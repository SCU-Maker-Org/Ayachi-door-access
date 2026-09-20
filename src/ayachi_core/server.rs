// Made by Han_feng

use super::{MAX_CONNECTIONS, NAME, PORT, ROUNDS, SALT, USERS, WWW_AUTHENTICATE};
use super::door::{Door, DoorSignal, DoorState};
use base64::Engine;
use embassy_net::Stack;
use pbkdf2::pbkdf2_hmac_array;
use pbkdf2::sha2::Sha256;
use picoserve::extract::{FromRequestParts, Query};
use picoserve::request::RequestParts;
use picoserve::response::{NoContent, Response, StatusCode};
use picoserve::routing::{PathRouter, get};
use picoserve::{AppBuilder, Router};
use serde::Deserialize;

// Structs
pub struct AyachiServer{
    pub door: &'static Door<'static>,
    pub door_signal: &'static DoorSignal
}

struct Authorizer;

#[derive(Deserialize)]
struct SetOpenQuery {
    state: u8
}

// Impls
impl AppBuilder for AyachiServer {
    type PathRouter = impl PathRouter;

    fn build_app(self) -> Router<Self::PathRouter> {
        Router::new()
            .route("/ciallo", get(|| async { "Ciallo～(∠·ω< )⌒★" }))
            .route("/", get(|_: Authorizer| async {
                Response::ok(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/index.html")))
                    .with_content_type("text/html; charset=utf-8")
            }))
            .route("/status", get(|| async {
                Response::ok(match self.door.status() {
                    DoorState::Open => "open",
                    DoorState::Lock => "lock",
                })
            }))
            .route("/setopen", get(move |_: Authorizer, Query(query): Query<SetOpenQuery>| async move {
                match query.state {
                    0 => {
                        self.door_signal.lock.signal(());
                        (StatusCode::OK, "Door set to locked")
                    },
                    1 => {
                        self.door_signal.open.signal(());
                        (StatusCode::OK, "Door set to opened")
                    },
                    _ => (StatusCode::BAD_REQUEST, "Invalid state")
                }
            }))
            .route("/open", get(|_: Authorizer| async {
                self.door_signal.open_once.signal(());
                "Successfully open door for once"
            }))
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

// Tasks
#[embassy_executor::task(pool_size = MAX_CONNECTIONS)]
pub async fn server_task(server: AyachiServer, stack: Stack<'static>) {
    static SERVER_CONFIG: picoserve::Config = picoserve::Config::const_default();
    let app = server.build_app();

    let mut http_buffer = [0; 2048];
    let mut rx_buffer = [0; 1024];
    let mut tx_buffer = [0; 1024];

    picoserve::Server::new(&app, &SERVER_CONFIG, &mut http_buffer)
        .listen_and_serve(NAME, stack, PORT, &mut rx_buffer, &mut tx_buffer)
        .await;
}