mod client;
pub mod projection;

use std::{sync::mpsc::TryRecvError, time::Duration};

use client::{ConnectionState, SubscriptionEvent, SubscriptionWorker};
use gpui::{
    App, Context, Entity, FocusHandle, Focusable, Render, Task, Window, WindowBounds, WindowKind,
    WindowOptions, actions, prelude::*, px,
};
use platform_title_bar::PlatformTitleBar;
use projection::AttentionRow;
use release_channel::ReleaseChannel;
use ui::{Label, prelude::*};
use util::ResultExt as _;
use workspace::client_side_decorations;

actions!(openagents, [OpenConvexInbox]);

pub fn init(cx: &mut App) {
    cx.on_action(|_: &OpenConvexInbox, cx| open_convex_inbox(cx));
}

pub struct ConvexInboxWindow {
    title_bar: Option<Entity<PlatformTitleBar>>,
    focus_handle: FocusHandle,
    connection: ConnectionState,
    status: String,
    rows: Vec<AttentionRow>,
    worker: Option<SubscriptionWorker>,
    _subscription_task: Option<Task<()>>,
}

impl ConvexInboxWindow {
    fn new(cx: &mut Context<Self>) -> Self {
        let title_bar = if cfg!(target_os = "macos") {
            None
        } else {
            Some(cx.new(|cx| PlatformTitleBar::new("convex-inbox-title-bar", cx)))
        };
        Self {
            title_bar,
            focus_handle: cx.focus_handle(),
            connection: ConnectionState::Connecting,
            status: "Authenticating with OpenAgents…".to_string(),
            rows: Vec::new(),
            worker: None,
            _subscription_task: None,
        }
    }

    fn start(entity: &Entity<Self>, cx: &mut App) {
        let session = omega_effectd::openagents_session(cx);
        let task = cx.spawn({
            let entity = entity.downgrade();
            async move |cx| {
                let source = match session.controller_token_source(cx).await {
                    Ok(source) => source,
                    Err(error) => {
                        entity
                            .update(cx, |this, cx| {
                                this.status = format!("OpenAgents sign-in required: {error}");
                                cx.notify();
                            })
                            .ok();
                        return;
                    }
                };
                let (worker, receiver) = match SubscriptionWorker::spawn(source) {
                    Ok(worker) => worker,
                    Err(error) => {
                        entity
                            .update(cx, |this, cx| {
                                this.status = format!("Convex client unavailable: {error:#}");
                                cx.notify();
                            })
                            .ok();
                        return;
                    }
                };
                if entity
                    .update(cx, |this, cx| {
                        this.worker = Some(worker);
                        this.status = "Connecting to Convex…".to_string();
                        cx.notify();
                    })
                    .is_err()
                {
                    return;
                }

                loop {
                    let mut changed = false;
                    loop {
                        match receiver.try_recv() {
                            Ok(event) => {
                                changed = true;
                                if entity
                                    .update(cx, |this, _| this.apply_event(event))
                                    .is_err()
                                {
                                    return;
                                }
                            }
                            Err(TryRecvError::Empty) => break,
                            Err(TryRecvError::Disconnected) => return,
                        }
                    }
                    if changed && entity.update(cx, |_, cx| cx.notify()).is_err() {
                        return;
                    }
                    cx.background_executor()
                        .timer(Duration::from_millis(100))
                        .await;
                }
            }
        });
        entity.update(cx, |this, _| this._subscription_task = Some(task));
    }

    fn apply_event(&mut self, event: SubscriptionEvent) {
        match event {
            SubscriptionEvent::Connection(connection) => {
                self.connection = connection;
                self.status = match connection {
                    ConnectionState::Connecting => "Reconnecting to Convex…".to_string(),
                    ConnectionState::Connected => "Live — official Convex Rust client".to_string(),
                };
            }
            SubscriptionEvent::Snapshot(rows) => {
                self.rows = rows;
                self.status = "Live — official Convex Rust client".to_string();
            }
            SubscriptionEvent::Failure(message) => {
                self.status = message;
            }
        }
    }

    fn render_row(row: &AttentionRow, cx: &App) -> impl IntoElement {
        let identity = row
            .identifier
            .as_deref()
            .unwrap_or(row.aggregate_id.as_str());
        let requests = row.pending_approval_count + row.pending_input_count;
        v_flex()
            .w_full()
            .gap_1()
            .px_4()
            .py_3()
            .border_b_1()
            .border_color(cx.theme().colors().border_variant)
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .gap_3()
                    .child(Label::new(row.label.clone()))
                    .child(
                        Label::new(row.attention_state.clone())
                            .size(ui::LabelSize::Small)
                            .color(ui::Color::Muted),
                    ),
            )
            .child(
                h_flex()
                    .gap_3()
                    .text_sm()
                    .text_color(cx.theme().colors().text_muted)
                    .child(identity.to_string())
                    .child(row.status.clone())
                    .child(format!("generation {}", row.generation as u64))
                    .when(requests > 0.0, |this| {
                        this.child(format!("{} awaiting response", requests as u64))
                    }),
            )
    }
}

impl Render for ConvexInboxWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let connection_copy = if self.connection == ConnectionState::Connected {
            "Connected"
        } else {
            "Connecting"
        };
        let content = v_flex()
            .id("omega-convex-inbox")
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(cx.theme().colors().editor_background)
            .child(
                v_flex()
                    .gap_1()
                    .px_4()
                    .py_3()
                    .border_b_1()
                    .border_color(cx.theme().colors().border)
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .child(Label::new("OpenAgents work inbox"))
                            .child(
                                Label::new(connection_copy)
                                    .size(ui::LabelSize::Small)
                                    .color(if self.connection == ConnectionState::Connected {
                                        ui::Color::Success
                                    } else {
                                        ui::Color::Muted
                                    }),
                            ),
                    )
                    .child(
                        Label::new(self.status.clone())
                            .size(ui::LabelSize::Small)
                            .color(ui::Color::Muted),
                    ),
            )
            .child(
                v_flex()
                    .id("omega-convex-inbox-rows")
                    .size_full()
                    .overflow_y_scroll()
                    .when(self.rows.is_empty(), |this| {
                        this.child(
                            v_flex()
                                .p_4()
                                .child(Label::new("No work shells in this workspace.")),
                        )
                    })
                    .children(self.rows.iter().map(|row| Self::render_row(row, cx))),
            );

        client_side_decorations(
            v_flex()
                .size_full()
                .text_color(cx.theme().colors().text)
                .children(self.title_bar.clone())
                .child(content),
            window,
            cx,
        )
    }
}

impl Focusable for ConvexInboxWindow {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

fn open_convex_inbox(cx: &mut App) {
    if let Some(existing) = cx
        .windows()
        .into_iter()
        .find_map(|window| window.downcast::<ConvexInboxWindow>())
    {
        existing
            .update(cx, |_, window, _| window.activate_window())
            .log_err();
        return;
    }

    let app_id = ReleaseChannel::global(cx).app_id();
    cx.open_window(
        WindowOptions {
            titlebar: Some(gpui::TitlebarOptions {
                title: Some("OpenAgents work inbox".into()),
                appears_transparent: true,
                traffic_light_position: Some(gpui::point(px(12.0), px(12.0))),
            }),
            focus: true,
            show: true,
            is_movable: true,
            kind: WindowKind::Normal,
            window_background: cx.theme().window_background_appearance(),
            app_id: Some(app_id.to_owned()),
            window_decorations: Some(gpui::WindowDecorations::Client),
            window_bounds: Some(WindowBounds::centered(gpui::size(px(760.0), px(620.0)), cx)),
            window_min_size: Some(gpui::size(px(420.0), px(320.0))),
            ..Default::default()
        },
        |_, cx| {
            let entity = cx.new(ConvexInboxWindow::new);
            ConvexInboxWindow::start(&entity, cx);
            entity
        },
    )
    .log_err();
}
