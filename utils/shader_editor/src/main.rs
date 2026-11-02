// --------------------------------------------------------------------------------------------------------------------
// Shader editor entry point. A shell around the library: parse the CLI, then either run the
// headless job or open the editor window.
//
// Which editor opens is the file's decision, not a flag's: a paired .glsl gets the macroquad
// preview it always got, a unified one gets the wgpu G-buffer preview. See dialect.rs.

use std::path::Path;

use macroquad::prelude::*;

use shader_editor::app::{poll_interval_s, App};
use shader_editor::dialect::{detect, Dialect};
use shader_editor::headless;
use shader_editor::unified_app::UnifiedApp;

fn window_conf() -> Conf {
    // a headless run still needs a GL context, so keep its window small and short-lived
    let headless = std::env::args().any(|a| a == "--check" || a == "--out");
    let (w, h, title) = if headless {
        (320, 200, "Shader Editor (headless)")
    } else {
        (1200, 768, "Shader Editor")
    };
    Conf { high_dpi: false, window_width: w, window_height: h, window_title: title.to_string(), ..Default::default() }
}

/// The two editors, behind one set of per-frame calls. They share the shape of the window and
/// nothing else: one drives a macroquad Material, the other a wgpu render pass.
enum Editor {
    Paired(App),
    Unified(UnifiedApp),
}

impl Editor {
    fn open(path: &Path) -> Result<Editor, String> {
        let src = std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
        match detect(&src).dialect {
            Dialect::Paired => App::new(path).map(Editor::Paired),
            Dialect::Unified => UnifiedApp::new(path).map(Editor::Unified),
        }
    }

    fn poll_reload(&mut self) {
        match self { Editor::Paired(a) => a.poll_reload(), Editor::Unified(a) => a.poll_reload() }
    }

    fn update_animated_uniforms(&mut self) {
        match self {
            Editor::Paired(a) => a.update_animated_uniforms(),
            Editor::Unified(a) => a.update_animated_uniforms(),
        }
    }

    fn draw_scene(&mut self) {
        match self { Editor::Paired(a) => a.draw_scene(), Editor::Unified(a) => a.draw_scene() }
    }

    fn ui(&mut self) {
        match self { Editor::Paired(a) => a.ui(), Editor::Unified(a) => a.ui() }
    }
}

#[macroquad::main(window_conf)]
async fn main() {
    let cli = match headless::parse_cli() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("ERROR: {e}");
            eprintln!("Usage: shader_editor <file.glsl> [--check | --out <file.png>] [--size WxH] [--time secs]");
            eprintln!("                     [--set n=v] [--channel lit|<role>|raw:N|all] [--tiles n] [--toggle NAME=on]");
            std::process::exit(2);
        }
    };

    if cli.is_headless() {
        std::process::exit(headless::run(cli).await);
    }

    let Some(path) = cli.glsl.clone() else {
        eprintln!("Usage: shader_editor <file.glsl> [--check | --out <file.png>]");
        std::process::exit(2);
    };

    // A startup failure gets its own window rather than a message in a terminal nobody is
    // watching: the tool is usually launched from an editor task, not a shell.
    let mut editor = match Editor::open(Path::new(&path)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            loop {
                clear_background(BLACK);
                egui_macroquad::ui(|ctx| {
                    egui::Window::new("Error").show(ctx, |ui| { ui.label(e.clone()); });
                });
                egui_macroquad::draw();
                next_frame().await;
            }
        }
    };

    let mut next_poll = get_time();
    loop {
        clear_background(Color::from_rgba(20, 20, 24, 255));

        let now = get_time();
        if now >= next_poll {
            editor.poll_reload();
            next_poll = now + poll_interval_s();
        }

        editor.update_animated_uniforms();
        editor.draw_scene();
        editor.ui();

        next_frame().await;
    }
}
