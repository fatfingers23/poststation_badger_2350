use core::cell::RefCell;
use core::sync::atomic::{compiler_fence, Ordering};
use defmt::{error, info};
use embassy_embedded_hal::shared_bus::asynch::spi::SpiDevice;
use embassy_rp::gpio::{Input, Output};
use embassy_sync::blocking_mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_time::{Delay, Duration, Instant, Timer};
use embedded_graphics::primitives::PrimitiveStyleBuilder;
use embedded_graphics::text::Text;
use embedded_graphics::{
    mono_font::{ascii::*, MonoTextStyle},
    pixelcolor::BinaryColor,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
};
use embedded_text::{
    alignment::HorizontalAlignment,
    style::{HeightMode, TextBoxStyleBuilder},
    TextBox,
};
use heapless::String;
use portable_atomic::AtomicBool;
use postcard_rpc::{header::VarHeader, server::Sender};
use template_icd::{LedState, SleepEndpoint, SleepMillis, SleptMillis};
use uc8151::asynch::Uc8151;
use uc8151::{LUT, WIDTH};

use crate::{
    app::{AppTx, Context, TaskContext},
    Spi0Bus,
};
pub static DISPLAY_HAS_CHANGED: AtomicBool = AtomicBool::new(false);
pub static SOME_TEXT: blocking_mutex::Mutex<CriticalSectionRawMutex, RefCell<String<120>>> =
    blocking_mutex::Mutex::new(RefCell::new(String::<120>::new()));

/// This is an example of a BLOCKING handler.
pub fn unique_id(context: &mut Context, _header: VarHeader, _arg: ()) -> u64 {
    context.unique_id
}

/// Also a BLOCKING handler
pub fn picoboot_reset(_context: &mut Context, _header: VarHeader, _arg: ()) {
    embassy_rp::rom_data::reboot(0x0002, 500, 0x0000, 0x0000);
    loop {
        // Wait for reset...
        compiler_fence(Ordering::SeqCst);
    }
}

/// Also a BLOCKING handler
pub fn set_led(context: &mut Context, _header: VarHeader, arg: LedState) {
    //Update SOME_TEXT
    let _ = SOME_TEXT.lock(|ip| {
        ip.borrow_mut().clear();
        ip.borrow_mut().push_str("LED is ").unwrap();
        ip.borrow_mut()
            .push_str(match arg {
                LedState::Off => "Off",
                LedState::On => "On",
            })
            .unwrap();
    });
    DISPLAY_HAS_CHANGED.store(true, Ordering::Relaxed);
    match arg {
        LedState::Off => context.led.set_low(),
        LedState::On => context.led.set_high(),
    }
}

pub fn get_led(context: &mut Context, _header: VarHeader, _arg: ()) -> LedState {
    match context.led.is_set_low() {
        true => LedState::Off,
        false => LedState::On,
    }
}

pub fn set_text(_context: &mut Context, _header: VarHeader, arg: &str) {
    SOME_TEXT.lock(|ip| {
        ip.borrow_mut().clear();
        ip.borrow_mut().push_str(arg).unwrap();
    });
    DISPLAY_HAS_CHANGED.store(true, Ordering::Relaxed);
}

/// This is a SPAWN handler
///
/// The pool size of three means we can have up to three of these requests "in flight"
/// at the same time. We will return an error if a fourth is requested at the same time
#[embassy_executor::task(pool_size = 3)]
pub async fn sleep_handler(
    _context: TaskContext,
    header: VarHeader,
    arg: SleepMillis,
    sender: Sender<AppTx>,
) {
    // We can send string logs, using the sender
    let _ = sender.log_str("Starting sleep...").await;
    let start = Instant::now();
    Timer::after_millis(arg.millis.into()).await;
    let _ = sender.log_str("Finished sleep").await;
    // Async handlers have to manually reply, as embassy doesn't support returning by value
    let _ = sender
        .reply::<SleepEndpoint>(
            header.seq_no,
            &SleptMillis {
                millis: start.elapsed().as_millis() as u16,
            },
        )
        .await;
}

#[embassy_executor::task]
pub async fn run_the_display(
    spi_bus: &'static Spi0Bus,
    cs: Output<'static>,
    dc: Output<'static>,
    busy: Input<'static>,
    reset: Output<'static>,
) {
    let spi_device = SpiDevice::new(&spi_bus, cs);
    let mut display = Uc8151::new(spi_device, dc, busy, reset, Delay);
    info!("Resetting display");
    display.reset().await;

    // Initialise display. Using the default LUT speed setting
    let test = display.setup(LUT::Medium).await;
    if test.is_err() {
        error!("Display setup failed");
    }

    let character_style = MonoTextStyle::new(&FONT_9X18_BOLD, BinaryColor::Off);
    let textbox_style = TextBoxStyleBuilder::new()
        .height_mode(HeightMode::FitToText)
        .alignment(HorizontalAlignment::Left)
        .paragraph_spacing(6)
        .build();

    // Bounding box for our text. Fill it with the opposite color so we can read the text.
    let static_text_bounds = Rectangle::new(Point::new(10, 50), Size::new(WIDTH, 0));
    static_text_bounds
        .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
        .draw(&mut display)
        .unwrap();

    // Crate static text
    // let text = "Hello BlueSky";
    // let text_box =
    //     TextBox::with_textbox_style(text, static_text_bounds, character_style, textbox_style);

    // // Draw the text box.
    // text_box.draw(&mut display).unwrap();
    let _ = display.update().await;

    loop {
        if DISPLAY_HAS_CHANGED.load(Ordering::Relaxed) {
            let some_text = SOME_TEXT.lock(|ip| ip.borrow().clone());
            let top_box = Rectangle::new(Point::new(0, 0), Size::new(WIDTH, 24));
            top_box
                .into_styled(
                    PrimitiveStyleBuilder::default()
                        .stroke_color(BinaryColor::Off)
                        .fill_color(BinaryColor::On)
                        .stroke_width(1)
                        .build(),
                )
                .draw(&mut display)
                .unwrap();

            Text::new(some_text.as_str(), Point::new(8, 16), character_style)
                .draw(&mut display)
                .unwrap();

            // Draw the counter text box.
            let _ = display.partial_update(top_box.try_into().unwrap()).await;
            DISPLAY_HAS_CHANGED.store(false, Ordering::Relaxed);
        }
        Timer::after(Duration::from_millis(100)).await;
    }
}
