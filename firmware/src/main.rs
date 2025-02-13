#![no_std]
#![no_main]
use core::sync::atomic::{compiler_fence, Ordering};

use app::AppTx;
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_executor::Spawner;
use embassy_rp::{
    bind_interrupts,
    block::ImageDef,
    gpio::{Level, Output},
    i2c::{self, I2c},
    peripherals::{I2C1, USB},
    usb,
};
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, mutex::Mutex};
use embassy_time::{Duration, Instant, Ticker};
use embassy_usb::{Config, UsbDevice};
use embedded_graphics::{image::Image, pixelcolor::BinaryColor, prelude::Point, prelude::*};
use postcard_rpc::{
    sender_fmt,
    server::{Dispatch, Sender, Server},
};
use ssd1306::{
    prelude::DisplayRotation, prelude::*, size::DisplaySize128x64, I2CDisplayInterface,
    Ssd1306Async,
};
use static_cell::StaticCell;
use tinybmp::Bmp;
type I2c1Bus = Mutex<NoopRawMutex, I2c<'static, I2C1, i2c::Async>>;

bind_interrupts!(pub struct Irqs {
    USBCTRL_IRQ => usb::InterruptHandler<USB>;
    I2C1_IRQ => i2c::InterruptHandler<I2C1>;
});

use {defmt_rtt as _, panic_probe as _};

pub mod app;
pub mod handlers;
pub mod io;

#[link_section = ".start_block"]
#[used]
pub static IMAGE_DEF: ImageDef = ImageDef::secure_exe();

// Program metadata for `picotool info`.
// This is needed if you are using picotool to flash the device
#[link_section = ".bi_entries"]
#[used]
pub static PICOTOOL_ENTRIES: [embassy_rp::binary_info::EntryAddr; 4] = [
    embassy_rp::binary_info::rp_program_name!(c"pc-usage-monitor"),
    embassy_rp::binary_info::rp_program_description!(c"Display your computer's usage on a Pico"),
    embassy_rp::binary_info::rp_cargo_version!(),
    embassy_rp::binary_info::rp_program_build_attribute!(),
];

fn usb_config(serial: &'static str) -> Config<'static> {
    let mut config = Config::new(0x16c0, 0x27DD);
    config.manufacturer = Some("Bailey Townsend");
    config.product = Some("pc-usage-monitor");
    config.serial_number = Some(serial);

    // Required for windows compatibility.
    // https://developer.nordicsemi.com/nRF_Connect_SDK/doc/1.9.1/kconfig/CONFIG_CDC_ACM_IAD.html#help
    config.device_class = 0xEF;
    config.device_sub_class = 0x02;
    config.device_protocol = 0x01;
    config.composite_with_iads = true;

    config
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    // SYSTEM INIT
    let p = embassy_rp::init(Default::default());

    //This generates a unique serial number for the device
    let unique_id: u64 = embassy_rp::otp::get_chipid().unwrap();
    static SERIAL_STRING: StaticCell<[u8; 16]> = StaticCell::new();
    let mut ser_buf = [b' '; 16];
    // This is a simple number-to-hex formatting
    unique_id
        .to_be_bytes()
        .iter()
        .zip(ser_buf.chunks_exact_mut(2))
        .for_each(|(b, chs)| {
            let mut b = *b;
            for c in chs {
                *c = match b >> 4 {
                    v @ 0..10 => b'0' + v,
                    v @ 10..16 => b'A' + (v - 10),
                    _ => b'X',
                };
                b <<= 4;
            }
        });
    let ser_buf = SERIAL_STRING.init(ser_buf);
    let ser_buf = core::str::from_utf8(ser_buf.as_slice()).unwrap();

    // USB/RPC INIT
    let driver = usb::Driver::new(p.USB, Irqs);
    let pbufs = app::PBUFS.take();
    let config = usb_config(ser_buf);

    //Set up the LED
    let mut led = Output::new(p.PIN_25, Level::Low);

    //Setup the I2c bus to connect to the SSD1306 display
    let i2c = I2c::new_async(p.I2C1, p.PIN_27, p.PIN_26, Irqs, i2c::Config::default());
    static I2C_BUS: StaticCell<I2c1Bus> = StaticCell::new();
    let i2c_bus = I2C_BUS.init(Mutex::new(i2c));

    // Set up for the SSD1206 display
    let i2c_dev = I2cDevice::new(i2c_bus);
    let interface = I2CDisplayInterface::new(i2c_dev);
    let mut display = Ssd1306Async::new(interface, DisplaySize128x64, DisplayRotation::Rotate0)
        .into_buffered_graphics_mode();
    let display_init_result = display.init().await;
    //If the display doesn't init since we do not have logging yet we will turn on the onboard LED
    if display_init_result.is_err() {
        led.set_high();
        loop {
            compiler_fence(Ordering::SeqCst);
        }
    }

    // Displays the boot screen
    let bmp_logo_data = include_bytes!("../pictures/logo-poststation.bmp");
    let bmp_logo = Bmp::<BinaryColor>::from_slice(bmp_logo_data).unwrap();
    let _ = Image::new(&bmp_logo, Point::new(0, 0)).draw(&mut display);
    display.flush().await.unwrap();

    let context = app::Context {
        unique_id,
        led,
        display,
    };

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
    // spawner.must_spawn(boot_screen(i2c_bus));

    let mut error_displaying = false;
    // Begin running!
    loop {
        // If the host disconnects, we'll return an error here.
        // If this happens, just wait until the host reconnects

        let _ = server.run().await;

        if !error_displaying {
            //If theres an error we only want it to show the first time. If it connects then this gets cleared on first display endpoint call
            let i2c_dev = I2cDevice::new(i2c_bus);
            let interface = I2CDisplayInterface::new(i2c_dev);
            let mut display =
                Ssd1306Async::new(interface, DisplaySize128x64, DisplayRotation::Rotate0)
                    .into_terminal_mode();
            let _ = display.clear().await;
            let _ = display.write_str("\nCannot connect :(").await;

            error_displaying = true;
        }
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
