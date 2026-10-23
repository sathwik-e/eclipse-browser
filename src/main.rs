mod app;
mod config;
mod events;
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

    let window = WindowBuilder::new()
        .with_title("Eclipse")
        .with_inner_size(LogicalSize::new(initial_width, initial_height))
        .with_min_inner_size(LogicalSize::new(860.0, 560.0))
        .with_titlebar_transparent(true)
        .with_fullsize_content_view(true)
        .with_title_hidden(true)
        .build(&event_loop)
        .expect("Failed to create native macOS window");

    let window = Arc::new(window);

    // Spawn memory tracking and hibernation ticker thread
    let ticker_proxy = proxy.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(3));
        if ticker_proxy.send_event(UserEvent::TickMemory).is_err() {
            break;
        }
    });

    let mut app = BrowserApp::new(
        window.clone(),
        proxy.clone(),
        (initial_width, initial_height),
    );

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
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
                app.resize(logical.width, logical.height);
            }

            Event::UserEvent(user_event) => match user_event {
                UserEvent::Ipc(msg) => {
                    app.handle_ipc(&msg);
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
                    app.create_tab(&url, true);
                }
                UserEvent::TickMemory => {
                    app.check_hibernation();
                    app.broadcast_state();
                }
            },

            _ => (),
        }
    });
}
