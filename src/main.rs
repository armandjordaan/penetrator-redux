//! Window setup and the frame loop.
//!
//! Everything of substance lives in the library crate; see `lib.rs` for the
//! module map. This file exists to own the window and to call
//! [`penetrator::app::App::frame`] once per frame.

use macroquad::prelude::*;
use penetrator::{app, config};

fn window_conf() -> Conf {
    Conf {
        window_title: "Penetrator Redux".to_owned(),
        // 2x the virtual canvas, which is a comfortable window on any modern
        // display and an exact integer scale.
        window_width: (config::VIRTUAL_W * 2.0) as i32,
        window_height: (config::VIRTUAL_H * 2.0) as i32,
        window_resizable: true,
        high_dpi: true,
        // Multisampling matters here: the entire game is thin diagonal lines.
        sample_count: 4,
        ..Default::default()
    }
}

#[macroquad::main(window_conf)]
async fn main() {
    let mut app = app::App::new().await;

    while !app.quit {
        app.frame();
        next_frame().await;
    }
}
