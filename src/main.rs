mod app;
mod config;
mod events;
#[cfg(target_os = "macos")]
mod macos_menu;
mod max;
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
        // Opaque: menu.m paints a solid window background, so per-pixel
        // window transparency would only cost compositing work.
        .with_titlebar_transparent(true)
        .with_fullsize_content_view(true)
        .with_title_hidden(true)
        .with_titlebar_buttons_hidden(true)
        .with_movable_by_window_background(false)
        // A native tab bar would cover the custom chrome.
        .with_automatic_window_tabbing(false)
        .build(&event_loop)
        .expect("Failed to create native macOS window");

    let window = Arc::new(window);

    // The native main menu is what makes standard editing shortcuts (⌘C/⌘V),
    // Quit, Minimize and the app's own commands behave like a real macOS app.
    #[cfg(target_os = "macos")]
    {
        macos_menu::install(proxy.clone());
        macos_menu::install_edge_monitor(proxy.clone());
        macos_menu::install_navigation_hooks(proxy.clone());
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
    // Optional smoke-test link from "another app": opens a URL through the
    // same path as a link macOS hands Eclipse (a standalone Little Eclipse
    // window by default). Inert unless ECLIPSE_SMOKE_LITTLE_URL is set.
    if let Ok(url) = std::env::var("ECLIPSE_SMOKE_LITTLE_URL") {
        let url = url.trim();
        if !url.is_empty() {
            app.queue_external_links([url.to_string()]);
        }
    }

    event_loop.run(move |event, _, control_flow| {
        match event {
            // Internal pages first get a moment to save pending edits; the
            // loop exits once they have (see `ready_to_quit` below).
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                app.request_quit();
            }

            Event::WindowEvent {
                event: WindowEvent::Destroyed,
                ..
            } => {
                *control_flow = ControlFlow::Exit;
            }

            // Links other apps hand Eclipse (it is the default browser, or
            // an app asked for it by name): AppKit's handler for the "open
            // URL" Apple Event calls tao's application:openURLs:, also for a
            // link that launches the app (tao delivers it once the loop runs).
            // Anything but http(s) is ignored (app::external_link_route).
            Event::Opened { urls } => {
                app.queue_external_links(urls.into_iter().map(String::from));
            }

            // Also delivered when AppKit terminates the app (⌘Q).
            Event::LoopDestroyed => {
                app.shutdown();
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

            // The last event of every loop iteration: run deferred work
            // (chrome updates are coalesced to one per display frame) and
            // sleep until the next deadline. Deciding here, and only here,
            // keeps a WaitUntil from being overwritten by later events.
            Event::RedrawEventsCleared => {
                if app.ready_to_quit() {
                    *control_flow = ControlFlow::Exit;
                }
                if !matches!(
                    *control_flow,
                    ControlFlow::Exit | ControlFlow::ExitWithCode(_)
                ) {
                    *control_flow = match app.pump() {
                        Some(deadline) => ControlFlow::WaitUntil(deadline),
                        None => ControlFlow::Wait,
                    };
                }
            }

            Event::UserEvent(user_event) => match user_event {
                UserEvent::Ipc(msg) => {
                    app.handle_ipc(&msg);
                }
                UserEvent::TabIpc {
                    tab_id,
                    msg,
                    origin,
                    document,
                    from_little_window,
                } => {
                    app.handle_tab_ipc(tab_id, &msg, &origin, document, from_little_window);
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
                UserEvent::NewWindowRequested { url, popup } => {
                    app.handle_new_window(url, popup);
                }
                UserEvent::DownloadStarted {
                    seq,
                    file_name,
                    url,
                    path,
                    origin,
                } => {
                    app.handle_download_started(seq, file_name, url, path, origin);
                }
                UserEvent::DownloadCompleted {
                    seq,
                    url,
                    file_name,
                    path,
                    success,
                } => {
                    app.handle_download_completed(seq, url, file_name, path, success);
                }
                UserEvent::NavigationFailed {
                    tab_id,
                    committed,
                    domain,
                    code,
                    description,
                    url,
                } => {
                    app.handle_navigation_failed(tab_id, committed, &domain, code, description, url);
                }
                UserEvent::WebContentTerminated { tab_id } => {
                    app.handle_web_content_terminated(tab_id);
                }
                UserEvent::SidebarWebContentTerminated => {
                    app.handle_sidebar_terminated();
                }
                UserEvent::PageFlushed { tab_id } => {
                    app.handle_page_flushed(tab_id);
                }
                UserEvent::SidebarFlushed => {
                    app.handle_sidebar_flushed();
                }
                UserEvent::ExtensionDownloaded { ext_id, result } => {
                    app.handle_extension_downloaded(ext_id, result);
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
                UserEvent::CloseWindow => {
                    app.request_quit();
                }
            },

            _ => (),
        }
    });
}
