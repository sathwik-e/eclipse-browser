mod app;
mod config;
mod events;
#[cfg(target_os = "macos")]
mod macos_menu;
mod tab;
mod ui_assets;

use std::sync::Arc;
use std::time::Duration;
use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    platform::macos::WindowBuilderExtMacOS,
    window::WindowBuilder,
};

use crate::app::BrowserApp;
use crate::events::UserEvent;

fn main() {
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let initial_width = 1340.0;
    let initial_height = 860.0;

    // Hide default horizontal macOS traffic lights so the sidebar's vertical red/yellow/green controls are used
    let window = WindowBuilder::new()
        .with_title("Eclipse")
        .with_inner_size(LogicalSize::new(initial_width, initial_height))
        .with_min_inner_size(LogicalSize::new(860.0, 560.0))
        .with_transparent(true)
        .with_titlebar_transparent(true)
        .with_fullsize_content_view(true)
        .with_title_hidden(true)
        .with_titlebar_buttons_hidden(true)
        .with_movable_by_window_background(false)
        .build(&event_loop)
        .expect("Failed to create native macOS window");

    let window = Arc::new(window);

    // The native main menu is what makes standard editing shortcuts (⌘C/⌘V),
    // Quit, Minimize and the app's own commands behave like a real macOS app.
    #[cfg(target_os = "macos")]
    {
        macos_menu::install(proxy.clone());
        macos_menu::install_edge_monitor(proxy.clone());
    }

    // Auto-quit hook for automated smoke tests: never active unless the
    // environment explicitly asks for it.
    if let Ok(milliseconds) = std::env::var("ECLIPSE_SMOKE_TEST_MS")
        && let Ok(milliseconds) = milliseconds.parse::<u64>()
    {
        let smoke_proxy = proxy.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(milliseconds));
            let _ = smoke_proxy.send_event(UserEvent::CloseWindow);
        });
    }

    let ticker_proxy = proxy.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(10));
            if ticker_proxy.send_event(UserEvent::TickMemory).is_err() {
                break;
            }
        }
    });

    let mut app = BrowserApp::new(
        window.clone(),
        proxy.clone(),
        (initial_width, initial_height),
    );

    // Optional smoke-test navigation: opens a normal tab through the same code
    // path as a user action. Inert unless ECLIPSE_SMOKE_URL is set.
    if let Ok(url) = std::env::var("ECLIPSE_SMOKE_URL") {
        let url = url.trim();
        if !url.is_empty() {
            app.create_tab(url, true);
        }
    }

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            }
            | Event::WindowEvent {
                event: WindowEvent::Destroyed,
                ..
            } => {
                *control_flow = ControlFlow::Exit;
            }

            Event::WindowEvent {
                event: WindowEvent::Resized(physical_size),
                ..
            } => {
                let factor = window.scale_factor();
                let logical = physical_size.to_logical::<f64>(factor);
                #[cfg(target_os = "macos")]
                {
                    let is_full = crate::macos_menu::is_fullscreen();
                    app.set_fullscreen(is_full);
                }
                app.resize(logical.width, logical.height);
            }

            Event::MainEventsCleared => {
                // Coalesce chrome updates to one per display frame instead of
                // one per page event/IPC message.
                if let Some(deadline) = app.pump_state() {
                    *control_flow = ControlFlow::WaitUntil(deadline);
                }
            }

            Event::UserEvent(user_event) => match user_event {
                UserEvent::Ipc(msg) => {
                    app.handle_ipc(&msg);
                }
                UserEvent::TabIpc { tab_id, msg } => {
                    app.handle_tab_ipc(tab_id, &msg);
                }
                UserEvent::PageStarted { tab_id, url } => {
                    app.handle_page_started(tab_id, url);
                }
                UserEvent::PageFinished { tab_id, url } => {
                    app.handle_page_finished(tab_id, url);
                }
                UserEvent::TitleChanged { tab_id, title } => {
                    app.handle_title_changed(tab_id, title);
                }
                UserEvent::NewWindowRequested { url } => {
                    app.handle_new_window(url);
                }
                UserEvent::DownloadStarted { file_name } => {
                    app.handle_download_started(file_name);
                }
                UserEvent::DownloadCompleted { file_name, success } => {
                    app.handle_download_completed(file_name, success);
                }
                UserEvent::MenuAction(action) => {
                    app.handle_menu_action(&action);
                }
                UserEvent::TickMemory => {
                    app.handle_maintenance_tick();
                }
                UserEvent::HoverExpandSidebar => {
                    app.hover_expand_sidebar();
                }
                UserEvent::HoverCollapseSidebar => {
                    app.hover_collapse_sidebar();
                }
                UserEvent::FinishSidebarCollapse => {
                    app.finish_sidebar_collapse();
                }
                UserEvent::CloseWindow => {
                    *control_flow = ControlFlow::Exit;
                }
            },

            _ => (),
        }
    });
}
