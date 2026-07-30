use crate::multibuffer_hint::MultibufferHint;
use client::{Client, UserStore, zed_urls};
use cloud_api_types::Plan;
use db::kvp::KeyValueStore;
use fs::Fs;
use gpui::{
    Action, AnyElement, App, AppContext, AsyncWindowContext, Context, Entity, EventEmitter,
    FocusHandle, Focusable, Global, IntoElement, KeyContext, Render, ScrollHandle, SharedString,
    Subscription, Task, WeakEntity, Window, actions,
};
use notifications::status_toast::StatusToast;
use project::{AgentRegistryStore, agent_server_store::AllAgentServersSettings};
use schemars::JsonSchema;
use serde::Deserialize;
use settings::{SettingsStore, VsCodeSettingsSource};
#[cfg(not(debug_assertions))]
use std::any::TypeId;
use std::sync::Arc;
use ui::{
    Divider, KeyBinding, ParentElement as _, StatefulInteractiveElement, Vector, VectorName,
    WithScrollbar as _, prelude::*, rems_from_px,
};

use omega_actions::{OpenEditorOnboarding, OpenOnboarding, dev::ResetOnboarding};
pub use workspace::welcome::ShowWelcome;
use workspace::welcome::WelcomePage;
use workspace::{
    Workspace, WorkspaceId,
    item::{Item, ItemEvent},
    notifications::NotifyResultExt as _,
    register_serializable_item, with_active_or_new_workspace,
};

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
#[action(namespace = omega, deprecated_aliases = ["zed::ImportVsCodeSettings"])]
#[serde(deny_unknown_fields)]
pub struct ImportVsCodeSettings {
    #[serde(default)]
    pub skip_prompt: bool,
}

/// Imports settings from Cursor editor.
#[derive(Copy, Clone, Debug, Default, PartialEq, Deserialize, JsonSchema, Action)]
#[action(namespace = omega, deprecated_aliases = ["zed::ImportCursorSettings"])]
#[serde(deny_unknown_fields)]
pub struct ImportCursorSettings {
    #[serde(default)]
    pub skip_prompt: bool,
}

pub use identity_startup::await_identity_ready;
#[cfg(any(test, feature = "test-support"))]
pub use identity_startup::install_test_identity_startup;

/// The retired first-run journey's completion key. omega#164 removed that
/// journey — startup provisions the identity silently in the background — so
/// the key is written by nothing and retained only so a debug reset still
/// clears records older profiles may hold. The cfg matches its one consumer,
/// `onboarding_completion_keys`; without it a release build flags the
/// constant as dead code.
#[cfg(any(debug_assertions, test))]
const IDENTITY_ONBOARDING_COMPLETION_KEY: &str = "omega_identity_onboarding_completion_v1";
const EDITOR_ONBOARDING_COMPLETION_KEY: &str = "omega_editor_onboarding_completion_v1";
const EDITOR_ONBOARDING_COMPLETION_SCHEMA: &str =
    "openagents.omega.editor-onboarding-completion.v1";

#[cfg(any(debug_assertions, test))]
fn onboarding_completion_keys() -> &'static [&'static str] {
    &[
        IDENTITY_ONBOARDING_COMPLETION_KEY,
        EDITOR_ONBOARDING_COMPLETION_KEY,
    ]
}

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

    // Handle OpenOnboarding and OpenEditorOnboarding with the same body.
    // Do not forward one action to the other via App::dispatch_action: that
    // nests a window update while the first action is still dispatching and
    // fails silently (Help → Editor Onboarding / Welcome Return to Onboarding).
    cx.on_action(|_: &OpenOnboarding, cx| open_editor_onboarding(cx));
    cx.on_action(|_: &OpenEditorOnboarding, cx| open_editor_onboarding(cx));

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
                .detach_and_log_err(cx);
        });
    });

    #[cfg(debug_assertions)]
    cx.on_action(|_: &ResetOnboarding, cx| {
        reset_onboarding_completion_records(cx);
    });
    #[cfg(not(debug_assertions))]
    {
        cx.on_action(|_: &ResetOnboarding, cx| {
            zlog::warn!("dev::ResetOnboarding is only available in debug builds");
            let _ = cx;
        });
        command_palette_hooks::CommandPaletteFilter::update_global(cx, |filter, _cx| {
            filter.hide_action_types(&[TypeId::of::<ResetOnboarding>()]);
        });
    }

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

fn open_editor_onboarding(cx: &mut App) {
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
            .detach_and_log_err(cx);
    });
}

#[cfg(debug_assertions)]
fn reset_onboarding_completion_records(cx: &mut App) {
    let kvp = KeyValueStore::global(cx);
    let keys = onboarding_completion_keys()
        .iter()
        .map(|key| (*key).to_string())
        .collect::<Vec<_>>();
    cx.spawn(async move |cx| {
        for key in keys {
            if let Err(error) = kvp.delete_kvp(key.clone()).await {
                zlog::error!("failed to clear onboarding completion `{key}`: {error:#}");
            }
        }
        let _ = cx.update(|cx| {
            with_active_or_new_workspace(cx, |workspace, _, cx| {
                let toast = StatusToast::new(
                    "Cleared onboarding completion records. Reopen via Help → Editor Onboarding or the command palette (cmd-shift-p / ctrl-shift-p, not the file picker).",
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
                workspace.toggle_status_toast(toast, cx);
            });
        });
    })
    .detach();
}

// omega#164, owner direction 2026-07-29: the first-run identity onboarding
// journey is removed, not hidden. `show_onboarding_view`, the first-run window
// mode, and the startup completion handoff are gone; a fresh profile gets its
// Nostr identity from the silent background provisioning in
// `identity_startup`, and this page remains only as the explicit editor-setup
// journey (Help → Editor Onboarding).

struct Onboarding {
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

fn editor_setup_completion(identity_ref: &str) -> OnboardingCompletion<'_> {
    OnboardingCompletion {
        schema: EDITOR_ONBOARDING_COMPLETION_SCHEMA,
        identity_ref,
    }
}

impl Onboarding {
    fn new_editor_setup(workspace: &Workspace, window: &mut Window, cx: &mut App) -> Entity<Self> {
        Self::new(workspace, window, cx)
    }

    fn new(workspace: &Workspace, window: &mut Window, cx: &mut App) -> Entity<Self> {
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
        let completion = editor_setup_completion(identity_ref.as_str());
        let Ok(completion) = serde_json::to_string(&completion) else {
            self.finish_error = Some("Could not record identity setup completion.".into());
            cx.notify();
            return;
        };
        let kvp = KeyValueStore::global(cx);
        self.finish_error = None;
        self.finish_task = Some(cx.spawn(async move |this, cx| {
            let result = kvp
                .write_kvp(EDITOR_ONBOARDING_COMPLETION_KEY.to_string(), completion)
                .await;
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

            this.update(cx, |this, cx| {
                this.finish_task = None;
                telemetry::event!("Finish Setup");
                go_to_welcome_page(cx);
            })
            .ok();
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

    fn render_page(&mut self, compact: bool, cx: &mut Context<Self>) -> AnyElement {
        crate::basics_page::render_basics_page(
            &self.user_store,
            &self.identity_section,
            compact,
            cx,
        )
        .into_any_element()
    }
}

impl Render for Onboarding {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let compact = window.viewport_size().width < px(560.);
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
                            .when_else(compact, |this| this.p_4(), |this| this.p_12())
                            .gap_6()
                            .child(
                                div()
                                    .flex()
                                    .w_full()
                                    .gap_4()
                                    .when_else(
                                        compact,
                                        |this| this.flex_col().items_stretch(),
                                        |this| this.flex_row().items_center().justify_between(),
                                    )
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
                                            .when_else(
                                                compact,
                                                |this| this.items_stretch(),
                                                |this| this.items_end(),
                                            )
                                            .child({
                                                Button::new("finish_setup", "Finish Setup")
                                                    .style(ButtonStyle::Filled)
                                                    .size(ButtonSize::Medium)
                                                    .tab_index(1000_isize)
                                                    .when_else(
                                                        compact,
                                                        |this| this.full_width(),
                                                        |this| this.width(rems_from_px(200.)),
                                                    )
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
                            .child(self.render_page(compact, cx)),
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
        let workspace_id = workspace.database_id()?;

        let db = persistence::OnboardingPagesDb::global(cx);
        Some(
            cx.background_spawn(
                async move { db.save_onboarding_page(item_id, workspace_id).await },
            ),
        )
    }

    fn should_serialize(&self, event: &Self::Event) -> bool {
        event == &ItemEvent::UpdateTab
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
    fn the_editor_setup_completion_names_its_schema_and_identity() {
        assert_eq!(
            EDITOR_ONBOARDING_COMPLETION_KEY,
            "omega_editor_onboarding_completion_v1"
        );

        let identity_ref = "identity:test";
        let editor_record =
            serde_json::to_value(editor_setup_completion(identity_ref)).expect("editor completion");
        assert_eq!(editor_record["identity_ref"], identity_ref);
        assert_eq!(
            editor_record["schema"],
            "openagents.omega.editor-onboarding-completion.v1"
        );
    }

    /// The retired first-run journey's key stays in the reset list even though
    /// nothing writes it anymore, so a debug reset on an older profile clears
    /// the record that profile still holds.
    #[test]
    fn reset_onboarding_clears_both_completion_keys() {
        let keys = onboarding_completion_keys();
        assert_eq!(
            keys,
            &[
                "omega_identity_onboarding_completion_v1",
                "omega_editor_onboarding_completion_v1",
            ]
        );
        assert_eq!(OpenOnboarding.name(), "omega::OpenOnboarding");
        assert_eq!(OpenEditorOnboarding.name(), "omega::OpenEditorOnboarding");
        assert_eq!(ResetOnboarding.name(), "dev::ResetOnboarding");
    }
}
