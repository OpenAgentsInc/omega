use crate::multibuffer_hint::MultibufferHint;
use client::{Client, UserStore, zed_urls};
use cloud_api_types::Plan;
use db::kvp::KeyValueStore;
use fs::Fs;
use gpui::{
    Action, AnyElement, AnyWindowHandle, App, AppContext, AsyncWindowContext, Context, Entity,
    EventEmitter, FocusHandle, Focusable, Global, IntoElement, KeyContext, Render, ScrollHandle,
    SharedString, Subscription, Task, WeakEntity, Window, actions,
};
use notifications::status_toast::StatusToast;
use project::{AgentRegistryStore, agent_server_store::AllAgentServersSettings};
use schemars::JsonSchema;
use serde::Deserialize;
use settings::{SettingsStore, VsCodeSettingsSource};
use std::sync::Arc;
use ui::{
    Divider, KeyBinding, ParentElement as _, StatefulInteractiveElement, Vector, VectorName,
    WithScrollbar as _, prelude::*, rems_from_px,
};

pub use workspace::welcome::ShowWelcome;
use workspace::welcome::WelcomePage;
use workspace::{
    AppState, Workspace, WorkspaceId,
    dock::DockPosition,
    item::{Item, ItemEvent},
    notifications::NotifyResultExt as _,
    open_new, register_serializable_item, with_active_or_new_workspace,
};
use zed_actions::OpenOnboarding;

mod base_keymap_picker;
mod basics_page;
mod identity_controller;
mod identity_profile;
mod identity_section;
mod identity_startup;
pub mod multibuffer_hint;
mod secure_input;
mod theme_preview;

/// Imports settings from Visual Studio Code.
#[derive(Copy, Clone, Debug, Default, PartialEq, Deserialize, JsonSchema, Action)]
#[action(namespace = zed)]
#[serde(deny_unknown_fields)]
pub struct ImportVsCodeSettings {
    #[serde(default)]
    pub skip_prompt: bool,
}

/// Imports settings from Cursor editor.
#[derive(Copy, Clone, Debug, Default, PartialEq, Deserialize, JsonSchema, Action)]
#[action(namespace = zed)]
#[serde(deny_unknown_fields)]
pub struct ImportCursorSettings {
    #[serde(default)]
    pub skip_prompt: bool,
}

pub use identity_startup::await_identity_ready;

const IDENTITY_ONBOARDING_COMPLETION_KEY: &str = "omega_identity_onboarding_completion_v1";
const IDENTITY_ONBOARDING_COMPLETION_SCHEMA: &str =
    "openagents.omega.identity-onboarding-completion.v1";
const EDITOR_ONBOARDING_COMPLETION_KEY: &str = "omega_editor_onboarding_completion_v1";
const EDITOR_ONBOARDING_COMPLETION_SCHEMA: &str =
    "openagents.omega.editor-onboarding-completion.v1";

#[derive(serde::Serialize)]
struct OnboardingCompletion<'a> {
    schema: &'static str,
    identity_ref: &'a str,
}

actions!(
    onboarding,
    [
        /// Finish the onboarding process.
        Finish,
        /// Sign in while in the onboarding flow.
        SignIn,
        /// Open the user account page while in the onboarding flow.
        OpenAccount,
        /// Resets the welcome screen hints to their initial state.
        ResetHints
    ]
);

pub fn init(cx: &mut App) {
    cx.observe_new(|workspace: &mut Workspace, _, _cx| {
        workspace
            .register_action(|_workspace, _: &ResetHints, _, cx| MultibufferHint::set_count(0, cx));
    })
    .detach();

    cx.on_action(|_: &OpenOnboarding, cx| {
        with_active_or_new_workspace(cx, |workspace, window, cx| {
            workspace
                .with_local_workspace(window, cx, |workspace, window, cx| {
                    let existing = workspace
                        .active_pane()
                        .read(cx)
                        .items()
                        .find_map(|item| item.downcast::<Onboarding>());

                    if let Some(existing) = existing {
                        workspace.activate_item(&existing, true, true, window, cx);
                    } else {
                        let settings_page = Onboarding::new_editor_setup(workspace, window, cx);
                        workspace.add_item_to_active_pane(
                            Box::new(settings_page),
                            None,
                            true,
                            window,
                            cx,
                        )
                    }
                })
                .detach();
        });
    });

    cx.on_action(|_: &ShowWelcome, cx| {
        with_active_or_new_workspace(cx, |workspace, window, cx| {
            workspace
                .with_local_workspace(window, cx, |workspace, window, cx| {
                    let existing = workspace
                        .active_pane()
                        .read(cx)
                        .items()
                        .find_map(|item| item.downcast::<WelcomePage>());

                    if let Some(existing) = existing {
                        workspace.activate_item(&existing, true, true, window, cx);
                    } else {
                        let settings_page = cx
                            .new(|cx| WelcomePage::new(workspace.weak_handle(), false, window, cx));
                        workspace.add_item_to_active_pane(
                            Box::new(settings_page),
                            None,
                            true,
                            window,
                            cx,
                        )
                    }
                })
                .detach();
        });
    });

    cx.observe_new(|workspace: &mut Workspace, _window, _cx| {
        workspace.register_action(|_workspace, action: &ImportVsCodeSettings, window, cx| {
            let fs = <dyn Fs>::global(cx);
            let action = *action;

            let workspace = cx.weak_entity();

            window
                .spawn(cx, async move |cx: &mut AsyncWindowContext| {
                    handle_import_vscode_settings(
                        workspace,
                        VsCodeSettingsSource::VsCode,
                        action.skip_prompt,
                        fs,
                        cx,
                    )
                    .await
                })
                .detach();
        });

        workspace.register_action(|_workspace, action: &ImportCursorSettings, window, cx| {
            let fs = <dyn Fs>::global(cx);
            let action = *action;

            let workspace = cx.weak_entity();

            window
                .spawn(cx, async move |cx: &mut AsyncWindowContext| {
                    handle_import_vscode_settings(
                        workspace,
                        VsCodeSettingsSource::Cursor,
                        action.skip_prompt,
                        fs,
                        cx,
                    )
                    .await
                })
                .detach();
        });
    })
    .detach();

    base_keymap_picker::init(cx);

    register_serializable_item::<Onboarding>(cx);
    register_serializable_item::<WelcomePage>(cx);
}

pub fn show_onboarding_view(app_state: Arc<AppState>, cx: &mut App) -> Task<anyhow::Result<()>> {
    telemetry::event!("Onboarding Page Opened");
    open_new(
        Default::default(),
        app_state,
        cx,
        |workspace, window, cx| {
            {
                workspace.toggle_dock(DockPosition::Left, window, cx);
                let onboarding_page = Onboarding::new_first_run(workspace, window, cx);
                workspace.add_item_to_center(Box::new(onboarding_page.clone()), window, cx);

                window.focus(&onboarding_page.focus_handle(cx), cx);

                cx.notify();
            };
        },
    )
}

struct Onboarding {
    mode: OnboardingMode,
    workspace: WeakEntity<Workspace>,
    focus_handle: FocusHandle,
    user_store: Entity<UserStore>,
    scroll_handle: ScrollHandle,
    identity_section: Entity<identity_section::IdentitySection>,
    finish_task: Option<Task<()>>,
    finish_error: Option<SharedString>,
    _settings_subscription: Subscription,
    _identity_subscription: Subscription,
    _agent_registry_subscription: Option<Subscription>,
}

#[derive(Copy, Clone)]
enum OnboardingMode {
    FirstRun(AnyWindowHandle),
    EditorSetup,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum OnboardingJourney {
    FirstRun,
    EditorSetup,
}

impl OnboardingJourney {
    fn completion_key(self) -> &'static str {
        match self {
            Self::FirstRun => IDENTITY_ONBOARDING_COMPLETION_KEY,
            Self::EditorSetup => EDITOR_ONBOARDING_COMPLETION_KEY,
        }
    }

    fn completion_schema(self) -> &'static str {
        match self {
            Self::FirstRun => IDENTITY_ONBOARDING_COMPLETION_SCHEMA,
            Self::EditorSetup => EDITOR_ONBOARDING_COMPLETION_SCHEMA,
        }
    }

    fn is_serializable(self) -> bool {
        self == Self::EditorSetup
    }

    fn completion<'a>(self, identity_ref: &'a str) -> OnboardingCompletion<'a> {
        OnboardingCompletion {
            schema: self.completion_schema(),
            identity_ref,
        }
    }

    fn basics_page_mode(self) -> basics_page::BasicsPageMode {
        match self {
            Self::FirstRun => basics_page::BasicsPageMode::FirstRun,
            Self::EditorSetup => basics_page::BasicsPageMode::EditorSetup,
        }
    }
}

impl OnboardingMode {
    fn journey(self) -> OnboardingJourney {
        match self {
            Self::FirstRun(_) => OnboardingJourney::FirstRun,
            Self::EditorSetup => OnboardingJourney::EditorSetup,
        }
    }
}

impl Onboarding {
    fn new_first_run(workspace: &Workspace, window: &mut Window, cx: &mut App) -> Entity<Self> {
        Self::new(
            workspace,
            OnboardingMode::FirstRun(window.window_handle()),
            window,
            cx,
        )
    }

    fn new_editor_setup(workspace: &Workspace, window: &mut Window, cx: &mut App) -> Entity<Self> {
        Self::new(workspace, OnboardingMode::EditorSetup, window, cx)
    }

    fn new(
        workspace: &Workspace,
        mode: OnboardingMode,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        let font_family_cache = theme::FontFamilyCache::global(cx);

        let installed_agents = cx
            .global::<SettingsStore>()
            .get::<AllAgentServersSettings>(None)
            .clone();
        let client = Client::global(cx);
        let status = *client.status().borrow();
        let plan = workspace.user_store().read(cx).plan();
        let zed_agent_state = if status.is_signed_out()
            || matches!(
                status,
                client::Status::AuthenticationError | client::Status::ConnectionError
            ) {
            "signed_out"
        } else if status.is_signing_in() {
            "signing_in"
        } else {
            match plan {
                Some(Plan::ZedPro) => "pro",
                Some(Plan::ZedProTrial) => "trial",
                Some(Plan::ZedBusiness) => "business",
                Some(Plan::ZedVip) => "vip",
                Some(Plan::ZedStudent) => "student",
                Some(Plan::ZedFree) | None => "free",
            }
        };
        let agents_installed = basics_page::FEATURED_AGENTS
            .iter()
            .filter(|agent| installed_agents.contains_key(agent.id))
            .map(|agent| agent.id)
            .collect::<Vec<_>>();
        telemetry::event!(
            "Welcome Agent Setup Viewed",
            zed_agent = zed_agent_state,
            agents_installed = agents_installed,
        );

        let identity_section = identity_section::IdentitySection::new(0, window, cx);
        let agent_registry = AgentRegistryStore::try_global(cx);
        let onboarding = cx.new(|cx| {
            cx.spawn(async move |this, cx| {
                font_family_cache.prefetch(cx).await;
                this.update(cx, |_, cx| {
                    cx.notify();
                })
            })
            .detach();

            Self {
                mode,
                workspace: workspace.weak_handle(),
                focus_handle: cx.focus_handle(),
                scroll_handle: ScrollHandle::new(),
                identity_section: identity_section.clone(),
                finish_task: None,
                finish_error: None,
                user_store: workspace.user_store().clone(),
                _settings_subscription: cx
                    .observe_global::<SettingsStore>(move |_, cx| cx.notify()),
                _identity_subscription: cx.observe(&identity_section, |_, _, cx| cx.notify()),
                _agent_registry_subscription: agent_registry
                    .as_ref()
                    .map(|registry| cx.observe(registry, |_, _, cx| cx.notify())),
            }
        });
        identity_startup::onboarding_opened(cx);
        onboarding
    }

    fn on_finish(&mut self, _: &Finish, _: &mut Window, cx: &mut Context<Self>) {
        if self.finish_task.is_some() {
            return;
        }
        let Some(identity_ref) = self.identity_section.read(cx).ready_identity_ref() else {
            cx.notify();
            return;
        };
        let completion = self.mode.journey().completion(identity_ref.as_str());
        let Ok(completion) = serde_json::to_string(&completion) else {
            self.finish_error = Some("Could not record identity setup completion.".into());
            cx.notify();
            return;
        };
        let kvp = KeyValueStore::global(cx);
        let completion_key = self.mode.journey().completion_key();
        self.finish_error = None;
        self.finish_task = Some(cx.spawn(async move |this, cx| {
            let result = kvp.write_kvp(completion_key.to_string(), completion).await;
            if let Err(error) = result {
                zlog::error!("failed to record onboarding completion: {error:#}");
                this.update(cx, |this, cx| {
                    this.finish_task = None;
                    this.finish_error = Some("Could not finish setup. Please try again.".into());
                    cx.notify();
                })
                .ok();
                return;
            }

            let Ok(mode) = this.read_with(cx, |this, _| this.mode) else {
                return;
            };
            match mode {
                OnboardingMode::FirstRun(window_handle) => {
                    if let Err(error) = window_handle.update(cx, |_, window, cx| {
                        window.remove_window();
                        telemetry::event!("Finish Setup");
                        identity_startup::release_identity_waiters(cx);
                    }) {
                        zlog::error!("failed to close identity onboarding window: {error:#}");
                        this.update(cx, |this, cx| {
                            this.finish_task = None;
                            this.finish_error =
                                Some("Could not finish setup. Please try again.".into());
                            cx.notify();
                        })
                        .ok();
                        return;
                    }
                }
                OnboardingMode::EditorSetup => {
                    this.update(cx, |this, cx| {
                        this.finish_task = None;
                        telemetry::event!("Finish Setup");
                        identity_startup::release_identity_waiters(cx);
                        go_to_welcome_page(cx);
                    })
                    .ok();
                }
            }
        }));
    }

    fn handle_sign_in(&mut self, _: &SignIn, window: &mut Window, cx: &mut Context<Self>) {
        let client = Client::global(cx);
        let workspace = self.workspace.clone();

        window
            .spawn(cx, async move |mut cx| {
                client
                    .sign_in_with_optional_connect(true, &cx)
                    .await
                    .notify_workspace_async_err(workspace, &mut cx);
            })
            .detach();
    }

    fn handle_open_account(_: &OpenAccount, _: &mut Window, cx: &mut App) {
        cx.open_url(&zed_urls::account_url(cx))
    }

    fn render_page(&mut self, cx: &mut Context<Self>) -> AnyElement {
        crate::basics_page::render_basics_page(
            &self.user_store,
            &self.identity_section,
            self.mode.journey().basics_page_mode(),
            cx,
        )
        .into_any_element()
    }
}

impl Render for Onboarding {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .image_cache(gpui::retain_all("onboarding-page"))
            .key_context({
                let mut ctx = KeyContext::new_with_defaults();
                ctx.add("Onboarding");
                ctx.add("menu");
                ctx
            })
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(cx.theme().colors().editor_background)
            .on_action(cx.listener(Self::on_finish))
            .on_action(cx.listener(Self::handle_sign_in))
            .on_action(Self::handle_open_account)
            .on_action(cx.listener(|_, _: &menu::SelectNext, window, cx| {
                window.focus_next(cx);
                cx.notify();
            }))
            .on_action(cx.listener(|_, _: &menu::SelectPrevious, window, cx| {
                window.focus_prev(cx);
                cx.notify();
            }))
            .vertical_scrollbar_for(&self.scroll_handle, window, cx)
            .child(
                div()
                    .id("page-content")
                    .size_full()
                    .overflow_y_scroll()
                    .child(
                        v_flex()
                            .min_w_0()
                            .max_w(rems_from_px(780.))
                            .w_full()
                            .mx_auto()
                            .p_12()
                            .gap_6()
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_4()
                                    .justify_between()
                                    .child(
                                        h_flex()
                                            .gap_4()
                                            .child(Vector::square(VectorName::OmegaLogo, rems(2.5)))
                                            .child(
                                                v_flex()
                                                    .child(
                                                        Headline::new("Welcome to Omega")
                                                            .size(HeadlineSize::Small),
                                                    )
                                                    .child(
                                                        Label::new("Your last IDE.")
                                                            .color(Color::Muted)
                                                            .size(LabelSize::Small)
                                                            .italic(),
                                                    ),
                                            ),
                                    )
                                    .child(
                                        v_flex()
                                            .gap_1()
                                            .items_end()
                                            .child({
                                                Button::new("finish_setup", "Finish Setup")
                                                    .style(ButtonStyle::Filled)
                                                    .size(ButtonSize::Medium)
                                                    .width(rems_from_px(200.))
                                                    .disabled(
                                                        self.finish_task.is_some()
                                                            || !self
                                                                .identity_section
                                                                .read(cx)
                                                                .is_ready(),
                                                    )
                                                    .key_binding(KeyBinding::for_action_in(
                                                        &Finish,
                                                        &self.focus_handle,
                                                        cx,
                                                    ))
                                                    .on_click(|_, window, cx| {
                                                        window.dispatch_action(
                                                            Finish.boxed_clone(),
                                                            cx,
                                                        );
                                                    })
                                            })
                                            .when_some(self.finish_error.clone(), |this, error| {
                                                this.child(
                                                    Label::new(error)
                                                        .size(LabelSize::Small)
                                                        .color(Color::Error),
                                                )
                                            }),
                                    ),
                            )
                            .child(Divider::horizontal().color(ui::DividerColor::BorderVariant))
                            .child(self.render_page(cx)),
                    )
                    .track_scroll(&self.scroll_handle),
            )
    }
}

impl EventEmitter<ItemEvent> for Onboarding {}

impl Focusable for Onboarding {
    fn focus_handle(&self, _: &App) -> gpui::FocusHandle {
        self.focus_handle.clone()
    }
}

impl Item for Onboarding {
    type Event = ItemEvent;

    fn tab_content_text(&self, _detail: usize, _cx: &App) -> SharedString {
        "Onboarding".into()
    }

    fn telemetry_event_text(&self) -> Option<&'static str> {
        Some("Onboarding Page Opened")
    }

    fn show_toolbar(&self) -> bool {
        false
    }

    fn can_split(&self) -> bool {
        false
    }

    fn clone_on_split(
        &self,
        _workspace_id: Option<WorkspaceId>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Task<Option<Entity<Self>>> {
        Task::ready(None)
    }

    fn deactivated(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.identity_section.update(cx, |section, cx| {
            section.deactivate_and_reinspect(window, cx)
        });
    }

    fn workspace_deactivated(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.identity_section.update(cx, |section, cx| {
            section.deactivate_and_reinspect(window, cx)
        });
    }

    fn on_removed(&self, cx: &mut Context<Self>) {
        identity_startup::onboarding_closed(cx);
        self.identity_section
            .update(cx, |section, cx| section.clear_transient_state(cx));
    }

    fn to_item_events(event: &Self::Event, f: &mut dyn FnMut(workspace::item::ItemEvent)) {
        f(*event)
    }
}

fn go_to_welcome_page(cx: &mut App) {
    with_active_or_new_workspace(cx, |workspace, window, cx| {
        let Some((onboarding_id, onboarding_idx)) = workspace
            .active_pane()
            .read(cx)
            .items()
            .enumerate()
            .find_map(|(idx, item)| {
                let _ = item.downcast::<Onboarding>()?;
                Some((item.item_id(), idx))
            })
        else {
            return;
        };

        workspace.active_pane().update(cx, |pane, cx| {
            // Get the index here to get around the borrow checker
            let idx = pane.items().enumerate().find_map(|(idx, item)| {
                let _ = item.downcast::<WelcomePage>()?;
                Some(idx)
            });

            if let Some(idx) = idx {
                pane.activate_item(idx, true, true, window, cx);
            } else {
                let item = Box::new(
                    cx.new(|cx| WelcomePage::new(workspace.weak_handle(), false, window, cx)),
                );
                pane.add_item(item, true, true, Some(onboarding_idx), window, cx);
            }

            pane.remove_item(onboarding_id, false, false, window, cx);
        });
    });
}

pub async fn handle_import_vscode_settings(
    workspace: WeakEntity<Workspace>,
    source: VsCodeSettingsSource,
    skip_prompt: bool,
    fs: Arc<dyn Fs>,
    cx: &mut AsyncWindowContext,
) {
    use util::truncate_and_remove_front;

    let vscode_settings =
        match settings::VsCodeSettings::load_user_settings(source, fs.clone()).await {
            Ok(vscode_settings) => vscode_settings,
            Err(err) => {
                zlog::error!("{err:?}");
                let _ = cx.prompt(
                    gpui::PromptLevel::Info,
                    &format!("Could not find or load a {source} settings file"),
                    None,
                    &["OK"],
                );
                return;
            }
        };

    if !skip_prompt {
        let prompt = cx.prompt(
            gpui::PromptLevel::Warning,
            &format!(
                "Importing {} settings may overwrite your existing settings. \
                Will import settings from {}",
                vscode_settings.source,
                truncate_and_remove_front(&vscode_settings.path.to_string_lossy(), 128),
            ),
            None,
            &["Import", "Cancel"],
        );
        let result = cx.spawn(async move |_| prompt.await.ok()).await;
        if result != Some(0) {
            return;
        }
    };

    let Ok(result_channel) = cx.update(|_, cx| {
        let source = vscode_settings.source;
        let path = vscode_settings.path.clone();
        let result_channel = cx
            .global::<SettingsStore>()
            .import_vscode_settings(fs, vscode_settings);
        zlog::info!("Imported {source} settings from {}", path.display());
        result_channel
    }) else {
        return;
    };

    let result = result_channel.await;
    workspace
        .update_in(cx, |workspace, _, cx| match result {
            Ok(_) => {
                let confirmation_toast = StatusToast::new(
                    format!("Your {} settings were successfully imported.", source),
                    cx,
                    |this, _| {
                        this.icon(
                            Icon::new(IconName::Check)
                                .size(IconSize::Small)
                                .color(Color::Success),
                        )
                        .dismiss_button(true)
                    },
                );
                SettingsImportState::update(cx, |state, _| match source {
                    VsCodeSettingsSource::VsCode => {
                        state.vscode = true;
                    }
                    VsCodeSettingsSource::Cursor => {
                        state.cursor = true;
                    }
                });
                workspace.toggle_status_toast(confirmation_toast, cx);
            }
            Err(_) => {
                let error_toast = StatusToast::new(
                    "Failed to import settings. See log for details",
                    cx,
                    |this, _| {
                        this.icon(
                            Icon::new(IconName::Close)
                                .size(IconSize::Small)
                                .color(Color::Error),
                        )
                        .action("Open Log", |window, cx| {
                            window.dispatch_action(workspace::OpenLog.boxed_clone(), cx)
                        })
                        .dismiss_button(true)
                    },
                );
                workspace.toggle_status_toast(error_toast, cx);
            }
        })
        .ok();
}

#[derive(Default, Copy, Clone)]
pub struct SettingsImportState {
    pub cursor: bool,
    pub vscode: bool,
}

impl Global for SettingsImportState {}

impl SettingsImportState {
    pub fn global(cx: &App) -> Self {
        cx.try_global().cloned().unwrap_or_default()
    }
    pub fn update<R>(cx: &mut App, f: impl FnOnce(&mut Self, &mut App) -> R) -> R {
        cx.update_default_global(f)
    }
}

impl workspace::SerializableItem for Onboarding {
    fn serialized_item_kind() -> &'static str {
        "OnboardingPage"
    }

    fn cleanup(
        workspace_id: workspace::WorkspaceId,
        alive_items: Vec<workspace::ItemId>,
        _window: &mut Window,
        cx: &mut App,
    ) -> gpui::Task<gpui::Result<()>> {
        workspace::delete_unloaded_items(
            alive_items,
            workspace_id,
            "onboarding_pages",
            &persistence::OnboardingPagesDb::global(cx),
            cx,
        )
    }

    fn deserialize(
        _project: Entity<project::Project>,
        workspace: WeakEntity<Workspace>,
        workspace_id: workspace::WorkspaceId,
        item_id: workspace::ItemId,
        window: &mut Window,
        cx: &mut App,
    ) -> gpui::Task<gpui::Result<Entity<Self>>> {
        let db = persistence::OnboardingPagesDb::global(cx);
        window.spawn(cx, async move |cx| {
            if let Some(_) = db.get_onboarding_page(item_id, workspace_id)? {
                workspace.update_in(cx, |workspace, window, cx| {
                    Onboarding::new_editor_setup(workspace, window, cx)
                })
            } else {
                Err(anyhow::anyhow!("No onboarding page to deserialize"))
            }
        })
    }

    fn serialize(
        &mut self,
        workspace: &mut Workspace,
        item_id: workspace::ItemId,
        _closing: bool,
        _window: &mut Window,
        cx: &mut ui::Context<Self>,
    ) -> Option<gpui::Task<gpui::Result<()>>> {
        if !self.mode.journey().is_serializable() {
            return None;
        }
        let workspace_id = workspace.database_id()?;

        let db = persistence::OnboardingPagesDb::global(cx);
        Some(
            cx.background_spawn(
                async move { db.save_onboarding_page(item_id, workspace_id).await },
            ),
        )
    }

    fn should_serialize(&self, event: &Self::Event) -> bool {
        self.mode.journey().is_serializable() && event == &ItemEvent::UpdateTab
    }
}

mod persistence {
    use db::{
        query,
        sqlez::{domain::Domain, thread_safe_connection::ThreadSafeConnection},
        sqlez_macros::sql,
    };
    use workspace::WorkspaceDb;

    pub struct OnboardingPagesDb(ThreadSafeConnection);

    impl Domain for OnboardingPagesDb {
        const NAME: &str = stringify!(OnboardingPagesDb);

        const MIGRATIONS: &[&str] = &[
            sql!(
                        CREATE TABLE onboarding_pages (
                            workspace_id INTEGER,
                            item_id INTEGER UNIQUE,
                            page_number INTEGER,

                            PRIMARY KEY(workspace_id, item_id),
                            FOREIGN KEY(workspace_id) REFERENCES workspaces(workspace_id)
                            ON DELETE CASCADE
                        ) STRICT;
            ),
            sql!(
                        CREATE TABLE onboarding_pages_2 (
                            workspace_id INTEGER,
                            item_id INTEGER UNIQUE,

                            PRIMARY KEY(workspace_id, item_id),
                            FOREIGN KEY(workspace_id) REFERENCES workspaces(workspace_id)
                            ON DELETE CASCADE
                        ) STRICT;
                        INSERT INTO onboarding_pages_2 SELECT workspace_id, item_id FROM onboarding_pages;
                        DROP TABLE onboarding_pages;
                        ALTER TABLE onboarding_pages_2 RENAME TO onboarding_pages;
            ),
        ];
    }

    db::static_connection!(OnboardingPagesDb, [WorkspaceDb]);

    impl OnboardingPagesDb {
        query! {
            pub async fn save_onboarding_page(
                item_id: workspace::ItemId,
                workspace_id: workspace::WorkspaceId
            ) -> Result<()> {
                INSERT OR REPLACE INTO onboarding_pages(item_id, workspace_id)
                VALUES (?, ?)
            }
        }

        query! {
            pub fn get_onboarding_page(
                item_id: workspace::ItemId,
                workspace_id: workspace::WorkspaceId
            ) -> Result<Option<workspace::ItemId>> {
                SELECT item_id
                FROM onboarding_pages
                WHERE item_id = ? AND workspace_id = ?
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_run_and_editor_setup_have_independent_completion_versions() {
        let first_run = OnboardingJourney::FirstRun;
        let editor_setup = OnboardingJourney::EditorSetup;

        assert_eq!(
            first_run.completion_key(),
            "omega_identity_onboarding_completion_v1"
        );
        assert_eq!(
            editor_setup.completion_key(),
            "omega_editor_onboarding_completion_v1"
        );
        assert_ne!(first_run.completion_key(), editor_setup.completion_key());
        assert_ne!(
            first_run.completion_schema(),
            editor_setup.completion_schema()
        );

        let identity_ref = "identity:test";
        let first_run_record =
            serde_json::to_value(first_run.completion(identity_ref)).expect("first-run completion");
        let editor_record =
            serde_json::to_value(editor_setup.completion(identity_ref)).expect("editor completion");
        assert_eq!(first_run_record["identity_ref"], identity_ref);
        assert_eq!(editor_record["identity_ref"], identity_ref);
        assert_ne!(first_run_record["schema"], editor_record["schema"]);
    }

    #[test]
    fn only_editor_setup_is_serializable() {
        assert!(!OnboardingJourney::FirstRun.is_serializable());
        assert!(OnboardingJourney::EditorSetup.is_serializable());
        assert_eq!(
            OnboardingJourney::FirstRun.basics_page_mode(),
            basics_page::BasicsPageMode::FirstRun
        );
        assert_eq!(
            OnboardingJourney::EditorSetup.basics_page_mode(),
            basics_page::BasicsPageMode::EditorSetup
        );
    }
}
