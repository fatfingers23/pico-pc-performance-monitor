use crate::{
    app::{AppTx, Context, TaskContext},
    io::Cursor,
};
use core::fmt::Write;
use core::sync::atomic::{compiler_fence, Ordering};
use embassy_time::{Instant, Timer};
use embedded_graphics::{
    mono_font::{
        ascii::{self},
        MonoTextStyle, MonoTextStyleBuilder,
    },
    pixelcolor::BinaryColor,
    prelude::*,
    text::{Baseline, Text},
};
use icd::{LedState, SleepEndpoint, SleepMillis, SleptMillis, SysInfo};
use postcard_rpc::{header::VarHeader, server::Sender};

const TEXT_STYLE: MonoTextStyle<'static, BinaryColor> = MonoTextStyleBuilder::new()
    .font(&ascii::FONT_8X13)
    .text_color(BinaryColor::On)
    .build();

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

pub async fn set_screen_text<'a>(context: &mut Context, _header: VarHeader, arg: SysInfo<'a>) {
    context.display.clear_buffer();

    let buffer = &mut [0u8; 1024];
    let mut cursor = Cursor::new(buffer);

    let _ = write!(
        &mut cursor,
        "{}\nCPU:{} {}% \nRam:{}/{}\n\n{}",
        arg.host_name,
        arg.cpu_freq_text,
        arg.cpu_usage,
        arg.memory_usage,
        arg.total_memory,
        arg.scroll_text
    );
    Text::with_baseline(cursor.as_str(), Point::zero(), TEXT_STYLE, Baseline::Top)
        .draw(&mut context.display)
        .unwrap();
    let _ = context.display.flush().await;
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
