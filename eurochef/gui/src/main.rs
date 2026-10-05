#![warn(clippy::all, rust_2018_idioms)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide

use color_eyre::eyre::Result;

// When compiling natively:
#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<()> {
    use clap::Parser;
    use color_eyre::Report;

    #[derive(Parser, Debug)]
    struct Args {
        /// Input file
        file: Option<String>,

        /// hashcodes.h
        #[arg(long, short = 't')]
        hashcodes: Option<String>,

        /// Save a picture of the window to this PNG and quit (for testing)
        #[arg(long)]
        screenshot: Option<String>,

        /// The panel to show for --screenshot: info, text, textures, entities, scripts, maps
        #[arg(long, default_value = "maps")]
        panel: String,

        /// Write the loaded file again with its triggers as they were read (for testing)
        #[arg(long)]
        save_triggers: Option<String>,

        /// Frames to draw before the picture is taken
        #[arg(long, default_value_t = 90)]
        screenshot_after: u32,
    }
    let args = Args::parse();

    // Force enable backtraces
    std::env::set_var("RUST_BACKTRACE", "1");

    eurochef_gui::panic_dialog::setup();

    // Log to stdout (if you run with `RUST_LOG=debug`).
    tracing_subscriber::fmt::init();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280., 1024.])
            .with_app_id("eurochef")
            .with_drag_and_drop(true),
        depth_buffer: 24,
        multisampling: 0,
        ..Default::default()
    };
    let res = eframe::run_native(
        "Eurochef",
        native_options,
        Box::new(move |cc| {
            let mut app = eurochef_gui::EurochefApp::new(args.file, args.hashcodes, cc);
            if let Some(path) = args.save_triggers {
                app.save_triggers_request(path);
            }
            if let Some(path) = args.screenshot {
                app.screenshot_request(path, &args.panel, args.screenshot_after);
            }
            Ok(Box::new(app))
        }),
    );

    match res {
        Ok(()) => Ok(()),
        Err(e) => Err(Report::msg(e.to_string())),
    }
}

// when compiling to web using trunk.
#[cfg(target_arch = "wasm32")]
fn main() {
    // Make sure panics are logged using `console.error`.
    console_error_panic_hook::set_once();

    // Redirect tracing to console.log and friends:
    tracing_wasm::set_as_global_default();

    let web_options = eframe::WebOptions::default();

    wasm_bindgen_futures::spawn_local(async {
        eframe::WebRunner::new()
            .start(
                "the_canvas_id", // hardcode it
                web_options,
                Box::new(|cc| Ok(Box::new(eurochef_gui::EurochefApp::new(None, None, cc)))),
            )
            .await
            .expect("failed to start eframe");
    });
}
