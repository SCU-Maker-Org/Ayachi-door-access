// Made by Han_feng

use super::{MAX_CONNECTIONS, NAME, PORT, ROUNDS, SALT, USERS, WWW_AUTHENTICATE};
use super::door::{Door, DoorState};
use base64::Engine;
use edge_nal::io::Read;
use embassy_net::Stack;
use esp_hal::__macro_implementation::static_cell::StaticCell;
use pbkdf2::pbkdf2_hmac_array;
use pbkdf2::sha2::Sha256;
use picoserve::extract::{FromRequest, FromRequestParts, Query};
use picoserve::request::{RequestBody, RequestParts};
use picoserve::response::{IntoResponse, NoContent, Redirect, Response, ResponseWriter, StatusCode};
use picoserve::routing::{get, Layer, PathRouter, Next, post};
use picoserve::{AppBuilder, ResponseSent, Router};
use serde::Deserialize;
use crate::ayachi_core::system::{PartitionSlot, System, SystemError};
use crate::ayachi_core::utils::{CompactFormat, FixedString};

// Types
type AyachiApplication = Router<<AyachiServer as AppBuilder>::PathRouter>;

// Structs
pub struct AyachiServer {
    state: &'static AyachiServerState,
}

struct AyachiServerState {
    door: &'static Door<'static>,
    system: &'static System<'static>,
}

struct Authorizer;

struct Uploader;

struct MaintenanceLayer;

#[derive(Deserialize)]
struct SetOpenQuery {
    state: u8
}

#[derive(Deserialize)]
struct PartitionInfoQuery {
    slot: PartitionSlot
}

// Statics
static AYACHI_APPLICATION: StaticCell<AyachiApplication> = StaticCell::new();
static AYACHI_SERVER_STATE: StaticCell<AyachiServerState> = StaticCell::new();

// Impls
impl AyachiServer {
    pub fn init(door: &'static Door<'static>, system: &'static System<'static>) -> &'static AyachiApplication {
        AYACHI_APPLICATION.init(AyachiServer{ state: AYACHI_SERVER_STATE.init(AyachiServerState{ door, system }) }.build_app())
    }
}

impl AppBuilder for AyachiServer {
    type PathRouter = impl PathRouter;

    fn build_app(self) -> Router<Self::PathRouter> {
        Router::new()
            .route("/ciallo", get(|| async { "Ciallo～(∠·ω< )⌒★" }))
            .route("/", get(|_: Authorizer| async {
                Response::ok(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/index.html")))
                    .with_content_type("text/html; charset=utf-8")
            }))
            .route("/logo", get(|| async {
                Response::ok(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/pic/logo.webp")).as_slice())
                    .with_content_type("image/webp")
            }))
            .route("/favicon.ico", get(|| async {
                Response::ok(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/pic/favicon.ico")).as_slice())
                    .with_content_type("image/x-icon")
            }))
            .route("/status", get(|| async {
                Response::ok(match self.state.door.status() {
                    DoorState::Open => "open",
                    DoorState::Lock => "lock",
                })
            }))
            .route("/setopen", get(move |_: Authorizer, Query(query): Query<SetOpenQuery>| async move {
                match query.state {
                    0 => {
                        self.state.door.lock_signal.signal(());
                        (StatusCode::OK, "Door set to locked")
                    },
                    1 => {
                        self.state.door.open_signal.signal(());
                        (StatusCode::OK, "Door set to opened")
                    },
                    _ => (StatusCode::BAD_REQUEST, "Invalid state")
                }
            }))
            .route("/open", get(|_: Authorizer| async {
                self.state.door.open_once_signal.signal(());
                "Successfully open door for once"
            }))
            .nest("/system", Router::new()
                .route("/", get(|_: Authorizer| async {
                    Response::ok(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/system.html")))
                        .with_content_type("text/html; charset=utf-8")
                }))
                .nest("/activate", Router::new()
                    .route("/", post(|_: Authorizer| async {
                        let message = FixedString::<100>::new();
                        match self.state.system.activate_next() {
                            Ok(_) => Response::ok(message.write_format("Activating")),
                            Err(error) => Response::new(StatusCode::BAD_REQUEST, message.write_format(error))
                        }
                    }))
                    .route("/countdown", get(|| async {
                        FixedString::<10>::from(self.state.system.activate_countdown().into_compact_format())
                    }))
                )
                .route("/status", get(move |_: Authorizer, Query(PartitionInfoQuery{ slot }): Query<PartitionInfoQuery>| async move {
                    FixedString::<410>::from(self.state.system.get_partition_info(slot).into_compact_format())
                }))
                .route("/resetting", get(|| async {
                    Response::ok(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/resetting.html")))
                        .with_content_type("text/html; charset=utf-8")
                }))
                .route("/progress", get(|_: Authorizer| async {
                    FixedString::<80>::new().write_formats(format_args!("{}|{:?}", self.state.system.is_uploading(), self.state.system.get_process().into_compact_format()))
                }))
                .route("/maintenance", get(|_: Authorizer| async {
                    Response::ok(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/maintenance.html")))
                        .with_content_type("text/html; charset=utf-8")
                }))
                .route("/upload", post(|_: Authorizer, _: Uploader| async {
                    Response::ok("Upload successfully")
                }))
            )
            .layer(MaintenanceLayer)
            .with_state(self.state)
    }
}

impl Authorizer{
    fn authorize(username: &str, password: &str) -> bool {
        let password_hash = pbkdf2_hmac_array::<Sha256, 32>(password.as_bytes(), SALT.as_bytes(), ROUNDS);

        for user in USERS {
            if user.name == username && user.password_hash == password_hash {
                return true;
            }
        }

        false
    }
}

impl<'r, State> FromRequestParts<'r, State> for Authorizer {
    type Rejection = (StatusCode, (&'static str, &'static str), NoContent);

    async fn from_request_parts(_: &'r State, request_parts: &RequestParts<'r>) -> Result<Self, Self::Rejection> {
        const UNAUTHORIZED: (StatusCode, (&'static str, &'static str), NoContent) = (
            StatusCode::UNAUTHORIZED,
            ("WWW-Authenticate", WWW_AUTHENTICATE),
            NoContent,
        );

        let auth = request_parts.headers().get("authorization").ok_or(UNAUTHORIZED)?;
        let encoded_content = auth.as_str().map_err(|_| UNAUTHORIZED)?.strip_prefix("Basic").ok_or(UNAUTHORIZED)?;

        let mut content_buffer = [0; 128];
        let length = base64::prelude::BASE64_STANDARD.decode_slice(encoded_content.trim(), &mut content_buffer).map_err(|_| UNAUTHORIZED)?;

        let content = str::from_utf8(&content_buffer[..length]).map_err(|_| UNAUTHORIZED)?;
        let (username, password) = content.split_once(':').ok_or(UNAUTHORIZED)?;

        Self::authorize(username, password).then(|| Ok(Authorizer)).ok_or(UNAUTHORIZED)?
    }
}

impl<'r> FromRequest<'r, AyachiServerState> for Uploader {
    type Rejection = (StatusCode, FixedString<250>);

    async fn from_request<R: Read>(state: &'r AyachiServerState, _: RequestParts<'r>, request_body: RequestBody<'r, R>) -> Result<Self, Self::Rejection> {
        let content_length = request_body.content_length();
        let mut reader = request_body.reader().with_different_timeout(embassy_time::Duration::from_secs(3600)); // It's impossible to upload such a huge file or just wait for almost one hour

        match state.system.upload(content_length, async |buffer| {
            reader.read(buffer).await.map_err(|_| SystemError::WriteFailure)
        }).await {
            Ok(_) => Ok(Uploader),
            Err(error) => Err((StatusCode::BAD_REQUEST, error.into()))
        }
    }
}

impl<PathParameters> Layer<AyachiServerState, PathParameters> for MaintenanceLayer {
    type NextState = AyachiServerState;
    type NextPathParameters = PathParameters;

    async fn call_layer<'a, R: Read + 'a, NextLayer: Next<'a, R, Self::NextState, Self::NextPathParameters>, W: ResponseWriter<Error=R::Error>>(&self, next: NextLayer, state: &AyachiServerState, path_parameters: PathParameters, request_parts: RequestParts<'_>, response_writer: W) -> Result<ResponseSent, W::Error> {
        static MAINTENANCE_DEDICATED: &[&str] = &["/system/resetting", "/system/activate/countdown"];
        static MAINTENANCE_WHITELIST: &[&str] = & const {
            static WHITELIST_EXTENDED: &[&str] = &["/logo", "/favicon.ico"];

            let mut array = [""; MAINTENANCE_DEDICATED.len() + WHITELIST_EXTENDED.len()];
            let mut i = 0;
            while i < MAINTENANCE_DEDICATED.len() {
                array[i] = MAINTENANCE_DEDICATED[i];
                i += 1;
            }
            i = 0;
            while i < WHITELIST_EXTENDED.len() {
                array[MAINTENANCE_DEDICATED.len() + i] = WHITELIST_EXTENDED[i];
                i += 1;
            }

            array
        };

        let activating = state.system.activate_countdown().is_some();

        if activating && MAINTENANCE_WHITELIST.iter().find_map(|&white| (request_parts.path() == white).then_some(())).is_none() {
            return Redirect::to("/system/resetting").write_to(next.into_connection().await?, response_writer).await;
        }

        if !activating && MAINTENANCE_DEDICATED.iter().find_map(|&white| (request_parts.path() == white).then_some(())).is_some() {
            return Redirect::to("/system/").write_to(next.into_connection().await?, response_writer).await;
        }

        next.run(state, path_parameters, response_writer).await
    }
}

// Tasks
#[embassy_executor::task(pool_size = MAX_CONNECTIONS)]
pub async fn server_task(server_app: &'static AyachiApplication, stack: Stack<'static>) {
    static SERVER_CONFIG: picoserve::Config = picoserve::Config::const_default();

    let mut http_buffer = [0; 2048];
    let mut rx_buffer = [0; 1024];
    let mut tx_buffer = [0; 1024];

    picoserve::Server::new(server_app, &SERVER_CONFIG, &mut http_buffer)
        .listen_and_serve(NAME, stack, PORT, &mut rx_buffer, &mut tx_buffer)
        .await;
}