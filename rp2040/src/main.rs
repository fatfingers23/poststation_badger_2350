#![no_std]
#![no_main]

use app::AppTx;
use defmt::info;
use embassy_executor::Spawner;
use embassy_rp::{
    bind_interrupts,
    block::ImageDef,
    gpio::{Level, Output},
    peripherals::USB,
    usb,
};
use embassy_time::{Duration, Instant, Ticker};
use embassy_usb::{Config, UsbDevice};
use postcard_rpc::{
    sender_fmt,
    server::{Dispatch, Sender, Server},
};
use static_cell::StaticCell;

bind_interrupts!(pub struct Irqs {
    USBCTRL_IRQ => usb::InterruptHandler<USB>;
});

use {defmt_rtt as _, panic_probe as _};

pub mod app;
pub mod handlers;

#[link_section = ".start_block"]
#[used]
pub static IMAGE_DEF: ImageDef = ImageDef::secure_exe();

// // Program metadata for `picotool info`.
// // This isn't needed, but it's recomended to have these minimal entries.
#[link_section = ".bi_entries"]
#[used]
pub static PICOTOOL_ENTRIES: [embassy_rp::binary_info::EntryAddr; 4] = [
    embassy_rp::binary_info::rp_program_name!(c"Badger 2350"),
    embassy_rp::binary_info::rp_program_description!(c"Showcases the Badger 2350 with Poststation"),
    embassy_rp::binary_info::rp_cargo_version!(),
    embassy_rp::binary_info::rp_program_build_attribute!(),
];

fn usb_config(serial: &'static str) -> Config<'static> {
    let mut config = Config::new(0x16c0, 0x27DD);
    // config.manufacturer = Some("Sunny Brooke Development");
    // config.product = Some("rusty-badger-2350");
    config.manufacturer = Some("OneVariable");
    config.product = Some("poststation-pico");
    config.max_power = 100;
    config.max_packet_size_0 = 64;
    config.serial_number = Some(serial);

    // Required for windows compatibility.
    // https://developer.nordicsemi.com/nRF_Connect_SDK/doc/1.9.1/kconfig/CONFIG_CDC_ACM_IAD.html#help
    // config.device_class = 0xEF;
    // config.device_sub_class = 0x02;
    // config.device_protocol = 0x01;
    // config.composite_with_iads = true;

    config
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    // SYSTEM INIT
    info!("Start");
    let mut p = embassy_rp::init(Default::default());
    let mut led = Output::new(p.PIN_25, Level::Low);

    // let mut user_led = Output::new(p.PIN_25, Level::Low);

    // user_led.set_high();
    static SERIAL_STRING: StaticCell<[u8; 16]> = StaticCell::new();

    let mut ser_buf = [b' '; 16];
    //Rp 2350 does not have a embassy function to get the flash id for now so doing a static serial number
    let static_serial_number = "12345678";

    // ser_buf.copy_from_slice(static_serial_number.as_bytes());
    // let ser_buf = SERIAL_STRING.init(ser_buf);

    // let ser_buf = core::str::from_utf8(ser_buf.as_slice()).unwrap();

    // USB/RPC INIT
    let driver = usb::Driver::new(p.USB, Irqs);
    let pbufs: &mut postcard_rpc::server::impls::embassy_usb_v0_3::PacketBuffers =
        app::PBUFS.take();
    let config = usb_config(static_serial_number);

    let context = app::Context {
        unique_id: 123,
        // led,
    };
    led.set_high();

    let (device, tx_impl, rx_impl) =
        app::STORAGE.init_poststation(driver, config, pbufs.tx_buf.as_mut_slice());

    let dispatcher = app::MyApp::new(context, spawner.into());
    let vkk = dispatcher.min_key_len();

    let mut server: app::AppServer = Server::new(
        tx_impl,
        rx_impl,
        pbufs.rx_buf.as_mut_slice(),
        dispatcher,
        vkk,
    );

    let sender = server.sender();
    // We need to spawn the USB task so that USB messages are handled by
    // embassy-usb
    spawner.must_spawn(usb_task(device));
    spawner.must_spawn(logging_task(sender));

    // Begin running!
    loop {
        // If the host disconnects, we'll return an error here.
        // If this happens, just wait until the host reconnects
        let _ = server.run().await;
    }
}

/// This handles the low level USB management
#[embassy_executor::task]
pub async fn usb_task(mut usb: UsbDevice<'static, app::AppDriver>) {
    usb.run().await;
}

/// This task is a "sign of life" logger
#[embassy_executor::task]
pub async fn logging_task(sender: Sender<AppTx>) {
    let mut ticker = Ticker::every(Duration::from_secs(3));
    let start = Instant::now();
    loop {
        ticker.next().await;
        let _ = sender_fmt!(sender, "Uptime: {:?}", start.elapsed()).await;
    }
}
