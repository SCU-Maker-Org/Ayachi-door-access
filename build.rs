// Made by Han_feng

use pbkdf2::sha2::Sha256;
use pbkdf2::pbkdf2_hmac_array;
use serde::{Deserialize, Serialize};
use std::{env, fs, path};

const DEFAULT_CONFIG_PATH: &str = "config/server_config.toml";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=build.rs");

    // server config
    println!("cargo:rerun-if-env-changed=SERVER_CONFIG");

    let config_file_path = env::var("SERVER_CONFIG").map_or_else(|error| {
        println!("cargo:warning=Failed to read SERVER_CONFIG environment variable: {}, using default path: {}", error, DEFAULT_CONFIG_PATH);
        DEFAULT_CONFIG_PATH.to_string()
    }, |path| {
        println!("Using server config: {}", path);
        path
    });
    println!("cargo:rerun-if-changed={}", config_file_path);

    let config_file = fs::read_to_string(config_file_path)?;
    let config = toml::from_str::<Config>(&config_file)?;

    let mut config_buffer = vec![
        "// Made by Han_feng".to_string(),
        String::new(),
        format!("pub const NAME: &\'static str = {:?};", config.device.name),
        format!("pub const HOSTNAME: &\'static str = {:?};", config.device.hostname),
        format!("pub const PORT: u16 = {};", config.device.port),
        format!("pub const WWW_AUTHENTICATE: &\'static str = {:?};", format!("Basic realm={:?}", format!("{} Login", config.device.name))),
        String::new(),
        format!("pub const DOOR_OPEN_ONCE_DELAY: u64 = {};", config.door.open_once_delay),
        String::new(),
        format!("pub const WIFI_SSID: &\'static str = {:?};", config.wifi.ssid),
        format!("pub const WIFI_PASSWORD: &\'static str = {:?};", config.wifi.password),
        format!("pub const WIFI_RETRY_DELAY: u64 = {};", config.wifi.retry_delay),
        String::new(),
        format!("pub const MAX_CONNECTIONS: usize = {};", config.server.max_connections),
        String::new(),
        format!("pub const SALT: &\'static str = {:?};", config.users.salt),
        format!("pub const ROUNDS: u32 = {};", config.users.rounds),
        String::new(),
        "pub struct User { pub name: &\'static str, pub password_hash: [u8; 32] }".to_string(),
        "pub const USERS: &[User] = &[".to_string(),
    ];

    config_buffer.extend(config.users.signed.into_iter().map(|user| format!("    User {{ name: {:?}, password_hash: {:?} }},", user.name, pbkdf2_hmac_array::<Sha256, 32>(user.password.as_bytes(), config.users.salt.as_bytes(), config.users.rounds))));

    config_buffer.push("];".to_string());

    let out_dir = env::var("OUT_DIR")?;
    fs::write(path::Path::new(&out_dir).join("server_config.rs"), config_buffer.join("\n"))?;

    Ok(())
}

#[derive(Serialize, Deserialize)]
struct Config {
    device: DeviceConfig,
    door: DoorConfig,
    wifi: WifiConfig,
    server: ServerConfig,
    users: Users,
}

#[derive(Serialize, Deserialize)]
struct DeviceConfig{
    name: String,
    hostname: String,
    port: u16,
}

#[derive(Serialize, Deserialize)]
struct DoorConfig{
    open_once_delay: u64,
}

#[derive(Serialize, Deserialize)]
struct WifiConfig {
    ssid: String,
    password: String,
    retry_delay: u64,
}

#[derive(Serialize, Deserialize)]
struct ServerConfig{
    max_connections: usize,
}

#[derive(Serialize, Deserialize)]
struct Users{
    salt: String,
    rounds: u32,
    signed: Vec<UserConfig>,
}

#[derive(Serialize, Deserialize)]
struct UserConfig {
    name: String,
    password: String,
}