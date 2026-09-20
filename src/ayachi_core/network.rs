// Made by Han_feng

use super::{MAX_CONNECTIONS, NAME};
use core::ops::{Deref, DerefMut};
use embassy_net::{Runner, Stack, StackResources};
use esp_hal::__macro_implementation::static_cell::StaticCell;
use esp_hal::peripherals::WIFI;
use esp_radio::wifi::sta::StationConfig;
use esp_radio::wifi::{AuthenticationMethodConfig, ControllerConfig, Interface, WifiController, WifiError};

// Structs
pub struct Network<'w>{
    wifi_controller: WifiController<'w>,
    pub stack: Stack<'w>,
}

// Statics
static NET_RESOURCES: StaticCell<StackResources<{ 4 + MAX_CONNECTIONS }>> = StaticCell::new();
static NETWORK: StaticCell<Network> = StaticCell::new();

// Impls
impl<'w> Network<'w>{
    pub fn init(wifi: WIFI<'w>) -> Result<(&'static mut Self, Runner<'w, Interface>), WifiError>{
        let wifi_controller = WifiController::new(wifi, ControllerConfig::default())?;

        let rng = esp_hal::rng::Rng::new();
        let mut dhcp_config = embassy_net::DhcpConfig::default();
        dhcp_config.hostname = NAME.try_into().map_err(|error| {
            defmt::warn!("DHCP hostname conversion failed, and will become None instead(message: {})", error);
        }).ok();

        let (stack, runner) = embassy_net::new(
            Interface::station(),
            embassy_net::Config::dhcpv4(dhcp_config),
            NET_RESOURCES.init(StackResources::new()),
            ((rng.random() as u64) << 32) | rng.random() as u64
        );

        Ok((NETWORK.init(Network { wifi_controller, stack }), runner))
    }

    pub fn connect_to(&mut self, name: &str, password: &str) -> Result<(), WifiError>{
        let wifi_config = StationConfig::default()
            .with_ssid(name.try_into()?)
            .with_authentication(AuthenticationMethodConfig::Wpa2Personal(password.try_into()?));

        self.wifi_controller.set_config(&esp_radio::wifi::Config::Station(wifi_config))
    }
}

impl<'w> Deref for Network<'w>{
    type Target = WifiController<'w>;

    fn deref(&self) -> &Self::Target {
        &self.wifi_controller
    }
}

impl<'w> DerefMut for Network<'w> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.wifi_controller
    }
}

// Tasks
#[embassy_executor::task]
pub async fn network_task(mut runner: Runner<'static, Interface>){
    runner.run().await;
}