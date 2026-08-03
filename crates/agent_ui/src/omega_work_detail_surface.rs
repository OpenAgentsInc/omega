use std::ops::Range;

use chrono::{SecondsFormat, Utc};
use editor::{Editor, EditorElement, EditorStyle};
use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    KeyContext, Render, Styled, TextStyle, UniformListScrollHandle, Window, prelude::*,
    uniform_list,
};
use omega_effectd::all_work_contract::{
    IntentRef, IsoTimestamp, SafeInteger, ShortText, SourceRef, WorkSnapshot,
};
use omega_work_detail::{
    BoundedWorkHistory, SubmitIntentDisposition, WorkBlock, WorkCanonicalEvent, WorkDetail,
    WorkDetailJournal, WorkDetailSourceState, WorkIntent, WorkIntentOutcome, WorkMutationKind,
    WorkMutationOperation, WorkPresentation,
};
use omega_work_index::WorkIndexItem;
use settings::Settings as _;
use sha2::{Digest as _, Sha256};
use theme_settings::ThemeSettings;
use ui::{
    Button, ButtonSize, ButtonStyle, Color, Icon, IconName, Label, LabelSize, ToggleButtonGroup,
    ToggleButtonGroupSize, ToggleButtonGroupStyle, ToggleButtonSimple, prelude::*,
};

#[derive(Clone, Debug)]
pub enum WorkDetailSurfaceEvent {
    SubmitIntent(WorkIntent),
    OpenSource(WorkIndexItem),
    PresentationChanged(WorkPresentation),
    JournalChanged,
}

pub struct WorkDetailSurface {
    focus_handle: FocusHandle,
    title_editor: Entity<Editor>,
    item: WorkIndexItem,
    detail: WorkDetail,
    history: BoundedWorkHistory,
    history_scroll: UniformListScrollHandle,
    editing_title: bool,
    command_menu_open: bool,
    next_intent_sequence: u64,
    status: Option<String>,
}

impl WorkDetailSurface {
    pub fn new(
        item: WorkIndexItem,
        detail: WorkDetail,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let title = detail.snapshot().summary.title.0.clone();
        let next_intent_sequence = u64::try_from(detail.records().len())
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        let title_editor = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_text(title, window, cx);
            editor
        });
        let history = detail.history(omega_work_detail::MAX_WORK_HISTORY_ROWS);
        Self {
            focus_handle: cx.focus_handle(),
            title_editor,
            item,
            detail,
            history,
            history_scroll: UniformListScrollHandle::new(),
            editing_title: false,
            command_menu_open: false,
            next_intent_sequence,
            status: None,
        }
    }

    pub fn title(&self) -> &str {
        &self.detail.snapshot().summary.title.0
    }

    pub fn work_ref(&self) -> &str {
        &self.detail.snapshot().summary.work_ref.0
    }

    pub fn presentation(&self) -> WorkPresentation {
        self.detail.presentation()
    }

    pub fn item(&self) -> &WorkIndexItem {
        &self.item
    }

    pub fn journal(&self) -> WorkDetailJournal {
        self.detail.journal()
    }

    pub fn snapshot(&self) -> &WorkSnapshot {
        self.detail.snapshot()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn replace_title_text_for_tests(
        &mut self,
        title: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.title_editor
            .update(cx, |editor, cx| editor.set_text(title, window, cx));
    }

    pub fn set_presentation(
        &mut self,
        presentation: WorkPresentation,
        emit: bool,
        cx: &mut Context<Self>,
    ) {
        if presentation == WorkPresentation::Issue && !self.has_issue_projection() {
            self.status = Some("No source-backed Issue projection is available.".into());
            cx.notify();
            return;
        }
        if self.detail.presentation() == presentation {
            return;
        }
        self.detail.set_presentation(presentation);
        if emit {
            cx.emit(WorkDetailSurfaceEvent::PresentationChanged(presentation));
            cx.emit(WorkDetailSurfaceEvent::JournalChanged);
        }
        cx.notify();
    }

    pub fn set_source_state(&mut self, state: WorkDetailSourceState, cx: &mut Context<Self>) {
        self.status = match &state {
            WorkDetailSourceState::Loading => Some("Loading the canonical Work snapshot…".into()),
            WorkDetailSourceState::Ready => None,
            WorkDetailSourceState::Offline => {
                Some("Offline · showing the last qualified source snapshot.".into())
            }
            WorkDetailSourceState::Error(message) => Some(format!("Source error · {message}")),
            WorkDetailSourceState::Conflict(message) => {
                Some(format!("Source conflict · {message}"))
            }
        };
        self.detail.set_source_state(state);
        cx.notify();
    }

    pub fn reconcile_snapshot(
        &mut self,
        snapshot: WorkSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), omega_work_detail::WorkDetailError> {
        self.detail.reconcile_source_snapshot(snapshot)?;
        if !self.has_issue_projection() && self.detail.presentation() == WorkPresentation::Issue {
            self.detail.set_presentation(WorkPresentation::Work);
            cx.emit(WorkDetailSurfaceEvent::PresentationChanged(
                WorkPresentation::Work,
            ));
        }
        self.history = self
            .detail
            .history(omega_work_detail::MAX_WORK_HISTORY_ROWS);
        if !self.editing_title {
            let title = self.detail.snapshot().summary.title.0.clone();
            self.title_editor.update(cx, |editor, cx| {
                editor.set_text(title, window, cx);
            });
        }
        self.status = None;
        self.detail.set_source_state(WorkDetailSourceState::Ready);
        cx.notify();
        Ok(())
    }

    fn has_issue_projection(&self) -> bool {
        self.detail
            .snapshot()
            .issue
            .as_ref()
            .is_some_and(Option::is_some)
    }

    fn toggle_command_menu(&mut self, cx: &mut Context<Self>) {
        self.command_menu_open = !self.command_menu_open;
        cx.notify();
    }

    pub fn admit_event(
        &mut self,
        event: WorkCanonicalEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), omega_work_detail::WorkDetailError> {
        self.detail.admit_event(event)?;
        self.item.summary = self.detail.snapshot().summary.clone();
        self.history = self
            .detail
            .history(omega_work_detail::MAX_WORK_HISTORY_ROWS);
        let title = self.detail.snapshot().summary.title.0.clone();
        self.title_editor
            .update(cx, |editor, cx| editor.set_text(title, window, cx));
        self.editing_title = false;
        self.status = Some("Accepted · canonical Event admitted by the source.".into());
        cx.emit(WorkDetailSurfaceEvent::JournalChanged);
        cx.notify();
        Ok(())
    }

    pub fn reject_intent(
        &mut self,
        intent_ref: &IntentRef,
        detail: String,
        cx: &mut Context<Self>,
    ) -> Result<(), omega_work_detail::WorkDetailError> {
        let short = ShortText::try_from(detail.clone())?;
        self.detail.reject_intent(intent_ref, short)?;
        self.history = self
            .detail
            .history(omega_work_detail::MAX_WORK_HISTORY_ROWS);
        self.status = Some(format!("Rejected · {detail}"));
        cx.emit(WorkDetailSurfaceEvent::JournalChanged);
        cx.notify();
        Ok(())
    }

    pub fn resolve_intent(
        &mut self,
        intent_ref: &IntentRef,
        outcome: WorkIntentOutcome,
        cx: &mut Context<Self>,
    ) -> Result<(), omega_work_detail::WorkDetailError> {
        self.detail.resolve_intent(intent_ref, outcome.clone())?;
        self.history = self
            .detail
            .history(omega_work_detail::MAX_WORK_HISTORY_ROWS);
        self.status = Some(intent_outcome_label(&outcome));
        cx.emit(WorkDetailSurfaceEvent::JournalChanged);
        cx.notify();
        Ok(())
    }

    fn start_title_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.detail.can_mutate(WorkMutationKind::Title) {
            return;
        }
        let title = self.detail.snapshot().summary.title.0.clone();
        self.title_editor.update(cx, |editor, cx| {
            editor.set_text(title, window, cx);
            editor.select_all(&editor::actions::SelectAll, window, cx);
            editor.focus_handle(cx).focus(window, cx);
        });
        self.editing_title = true;
        self.status = None;
        cx.notify();
    }

    fn cancel_title_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.editing_title {
            return;
        }
        let title = self.detail.snapshot().summary.title.0.clone();
        self.title_editor
            .update(cx, |editor, cx| editor.set_text(title, window, cx));
        self.editing_title = false;
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    fn submit_title_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.editing_title {
            return;
        }
        let requested = self.title_editor.read(cx).text(cx).trim().to_string();
        let Ok(title) = ShortText::try_from(requested) else {
            self.status = Some("Title must contain between 1 and 512 characters.".into());
            cx.notify();
            return;
        };
        let Some(capability) = self.detail.capability().cloned() else {
            self.status = Some("No source mutation authority is available.".into());
            cx.notify();
            return;
        };
        let revision = self.detail.snapshot().summary.revision.0;
        let sequence = self.next_intent_sequence;
        self.next_intent_sequence = self.next_intent_sequence.saturating_add(1);
        let mut work_digest = Sha256::new();
        work_digest.update(self.detail.snapshot().summary.work_ref.0.as_bytes());
        let work_digest = format!("{:x}", work_digest.finalize());
        let Some(work_digest) = work_digest.get(..16) else {
            self.status = Some("Could not derive the Work Intent digest.".into());
            cx.notify();
            return;
        };
        let intent_ref = match IntentRef::try_from(format!(
            "intent:omega:work-detail:{work_digest}:{revision}:{sequence}"
        )) {
            Ok(intent_ref) => intent_ref,
            Err(error) => {
                self.status = Some(format!("Could not construct Work Intent · {error}"));
                cx.notify();
                return;
            }
        };
        let intent = WorkIntent {
            intent_ref,
            work_ref: self.detail.snapshot().summary.work_ref.clone(),
            actor_ref: match SourceRef::try_from("principal:omega:local-owner".to_string()) {
                Ok(actor_ref) => actor_ref,
                Err(error) => {
                    self.status = Some(format!("Could not identify the local actor · {error}"));
                    cx.notify();
                    return;
                }
            },
            source_ref: capability.source_ref,
            expected_revision: SafeInteger(revision),
            target_generation: capability.generation,
            idempotency_key: match ShortText::try_from(format!(
                "idempotency.omega.work-detail.{work_digest}.{revision}.{sequence}"
            )) {
                Ok(key) => key,
                Err(error) => {
                    self.status = Some(format!("Could not construct idempotency key · {error}"));
                    cx.notify();
                    return;
                }
            },
            submitted_at: match IsoTimestamp::try_from(
                Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            ) {
                Ok(timestamp) => timestamp,
                Err(error) => {
                    self.status = Some(format!("Could not timestamp Work Intent · {error}"));
                    cx.notify();
                    return;
                }
            },
            operation: WorkMutationOperation::SetTitle { title },
        };
        let online = matches!(self.detail.source_state(), WorkDetailSourceState::Ready);
        match self.detail.submit_intent(intent.clone(), online) {
            Ok(SubmitIntentDisposition::Submitted) => {
                self.editing_title = false;
                self.status = Some("Pending · waiting for a canonical source Event.".into());
                self.history = self
                    .detail
                    .history(omega_work_detail::MAX_WORK_HISTORY_ROWS);
                self.focus_handle.focus(window, cx);
                cx.emit(WorkDetailSurfaceEvent::SubmitIntent(intent));
                cx.emit(WorkDetailSurfaceEvent::JournalChanged);
            }
            Ok(SubmitIntentDisposition::Reconciled(outcome)) => {
                self.status = Some(intent_outcome_label(&outcome));
                cx.emit(WorkDetailSurfaceEvent::JournalChanged);
            }
            Err(error) => {
                self.status = Some(format!("Intent refused · {error}"));
            }
        }
        cx.notify();
    }

    fn select_relative_block(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.detail.blocks().is_empty() {
            return;
        }
        let current = self
            .detail
            .selected_block()
            .and_then(|selected| {
                self.detail
                    .blocks()
                    .iter()
                    .position(|block| block.block_ref == selected.block_ref)
            })
            .unwrap_or(0);
        let next = current
            .saturating_add_signed(delta)
            .min(self.detail.blocks().len().saturating_sub(1));
        let block_ref = self.detail.blocks()[next].block_ref.clone();
        if self.detail.select_block(&block_ref) {
            cx.emit(WorkDetailSurfaceEvent::JournalChanged);
            cx.notify();
        }
    }

    fn select_block(&mut self, block_ref: SourceRef, cx: &mut Context<Self>) {
        if self.detail.select_block(&block_ref) {
            cx.emit(WorkDetailSurfaceEvent::JournalChanged);
            cx.notify();
        }
    }

    fn render_text_input(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = ThemeSettings::get_global(cx);
        let text_style = TextStyle {
            color: cx.theme().colors().text,
            font_family: settings.ui_font.family.clone(),
            font_features: settings.ui_font.features.clone(),
            font_fallbacks: settings.ui_font.fallbacks.clone(),
            font_size: rems(1.).into(),
            font_weight: settings.ui_font.weight,
            line_height: relative(1.3),
            ..Default::default()
        };
        EditorElement::new(
            &self.title_editor,
            EditorStyle {
                background: cx.theme().colors().editor_background,
                local_player: cx.theme().players().local(),
                text: text_style,
                ..Default::default()
            },
        )
    }

    fn render_history_rows(
        &mut self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let colors = cx.theme().colors();
        self.history.rows[range]
            .iter()
            .map(|row| {
                h_flex()
                    .id(("omega-work-activity-row", row.sequence))
                    .h(px(40.))
                    .px_3()
                    .gap_3()
                    .border_b_1()
                    .border_color(colors.border_variant)
                    .role(gpui::Role::ListItem)
                    .aria_label(row.label.clone())
                    .child(
                        div()
                            .w(px(112.))
                            .flex_none()
                            .text_size(px(11.))
                            .text_color(colors.text_placeholder)
                            .child(row.kind.label()),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(colors.text)
                            .child(row.reference.clone()),
                    )
                    .into_any_element()
            })
            .collect()
    }

    fn render_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let summary = &self.detail.snapshot().summary;
        let presentation = self.detail.presentation();
        let has_issue = self.has_issue_projection();
        v_flex()
            .gap_3()
            .pb_4()
            .border_b_1()
            .border_color(colors.border_variant)
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .gap_4()
                    .when(has_issue, |header| {
                        header.child(
                            ToggleButtonGroup::single_row(
                                "omega-work-presentation",
                                [
                                    ToggleButtonSimple::new(
                                        "Work",
                                        cx.listener(|this, _, _, cx| {
                                            this.set_presentation(WorkPresentation::Work, true, cx);
                                        }),
                                    ),
                                    ToggleButtonSimple::new(
                                        "Issue",
                                        cx.listener(|this, _, _, cx| {
                                            this.set_presentation(
                                                WorkPresentation::Issue,
                                                true,
                                                cx,
                                            );
                                        }),
                                    ),
                                ],
                            )
                            .style(ToggleButtonGroupStyle::Outlined)
                            .size(ToggleButtonGroupSize::Custom(rems_from_px(28.)))
                            .label_size(LabelSize::Small)
                            .auto_width()
                            .selected_index(match presentation {
                                WorkPresentation::Work => 0,
                                WorkPresentation::Issue => 1,
                            }),
                        )
                    })
                    .when(!has_issue, |header| {
                        header.child(
                            Label::new("Work")
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        )
                    })
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new("work-detail-commands", "Commands")
                                    .style(ButtonStyle::Subtle)
                                    .size(ButtonSize::Compact)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.toggle_command_menu(cx);
                                    })),
                            )
                            .child(
                                Button::new("open-work-source", "Open source")
                                    .style(ButtonStyle::Outlined)
                                    .size(ButtonSize::Compact)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        cx.emit(WorkDetailSurfaceEvent::OpenSource(
                                            this.item.clone(),
                                        ));
                                    })),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .items_start()
                    .gap_3()
                    .child(
                        v_flex()
                            .min_w_0()
                            .flex_1()
                            .gap_1()
                            .when(!self.editing_title, |header| {
                                header.child(
                                    div()
                                        .id("omega-work-detail-title")
                                        .role(gpui::Role::Heading)
                                        .aria_level(1)
                                        .text_size(px(22.))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(colors.text)
                                        .child(summary.title.0.clone()),
                                )
                            })
                            .when(self.editing_title, |header| {
                                header.child(
                                    h_flex()
                                        .h(px(38.))
                                        .w_full()
                                        .max_w(px(720.))
                                        .px_2()
                                        .gap_2()
                                        .rounded_md()
                                        .border_1()
                                        .border_color(colors.border_selected)
                                        .child(self.render_text_input(cx)),
                                )
                            })
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        Label::new(summary.work_ref.0.clone())
                                            .size(LabelSize::XSmall)
                                            .color(Color::Muted),
                                    )
                                    .when_some(
                                        self.detail
                                            .snapshot()
                                            .issue
                                            .as_ref()
                                            .and_then(Option::as_ref),
                                        |row, issue| {
                                            row.child(
                                                Label::new(format!(
                                                    "Issue {} · same Work identity",
                                                    issue.identifier.0
                                                ))
                                                .size(LabelSize::XSmall)
                                                .color(Color::Muted),
                                            )
                                        },
                                    ),
                            ),
                    )
                    .when(
                        self.detail.can_mutate(WorkMutationKind::Title) && !self.editing_title,
                        |row| {
                            row.child(
                                Button::new("edit-work-title", "Edit title")
                                    .style(ButtonStyle::Subtle)
                                    .size(ButtonSize::Compact)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.start_title_edit(window, cx);
                                    })),
                            )
                        },
                    )
                    .when(self.editing_title, |row| {
                        row.child(
                            h_flex()
                                .gap_1()
                                .child(
                                    Button::new("save-work-title", "Save")
                                        .style(ButtonStyle::Filled)
                                        .size(ButtonSize::Compact)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.submit_title_edit(window, cx);
                                        })),
                                )
                                .child(
                                    Button::new("cancel-work-title", "Cancel")
                                        .style(ButtonStyle::Subtle)
                                        .size(ButtonSize::Compact)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.cancel_title_edit(window, cx);
                                        })),
                                ),
                        )
                    }),
            )
            .when_some(summary.description.as_ref(), |header, description| {
                header.child(
                    div()
                        .max_w(px(800.))
                        .text_size(px(13.))
                        .text_color(colors.text_muted)
                        .child(description.0.clone()),
                )
            })
            .child(h_flex().gap_2().flex_wrap().children([
                field_chip("State", work_contract_label(&summary.state), cx),
                field_chip("Priority", work_contract_label(&summary.priority), cx),
                field_chip("Domain", work_contract_label(&summary.domain), cx),
                field_chip("Class", work_contract_label(&summary.work_class), cx),
            ]))
            .when_some(self.status.as_ref(), |header, status| {
                header.child(
                    h_flex()
                        .id("omega-work-detail-status")
                        .px_3()
                        .py_2()
                        .gap_2()
                        .rounded_md()
                        .bg(colors.elevated_surface_background)
                        .role(gpui::Role::Status)
                        .aria_label(status.clone())
                        .child(Icon::new(IconName::Info).color(Color::Muted))
                        .child(Label::new(status.clone()).size(LabelSize::Small)),
                )
            })
            .when(self.command_menu_open, |header| {
                header.child(
                    v_flex()
                        .id("omega-work-command-menu")
                        .debug_selector(|| "omega.omega.work-detail.command-menu".into())
                        .max_w(px(520.))
                        .rounded_lg()
                        .border_1()
                        .border_color(colors.border_variant)
                        .bg(colors.elevated_surface_background)
                        .p_2()
                        .gap_1()
                        .role(gpui::Role::Menu)
                        .aria_label("Work commands")
                        .when(self.detail.can_mutate(WorkMutationKind::Title), |menu| {
                            menu.child(
                                Button::new("work-command-edit-title", "E · Edit title")
                                    .style(ButtonStyle::Subtle)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.command_menu_open = false;
                                        this.start_title_edit(window, cx);
                                    })),
                            )
                        })
                        .child(
                            Button::new("work-command-open-source", "O · Open source")
                                .style(ButtonStyle::Subtle)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.command_menu_open = false;
                                    cx.emit(WorkDetailSurfaceEvent::OpenSource(this.item.clone()));
                                })),
                        )
                        .when(has_issue, |menu| {
                            menu.child(
                                Button::new("work-command-toggle-issue", "I · Toggle Work / Issue")
                                    .style(ButtonStyle::Subtle)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.command_menu_open = false;
                                        let next = match this.detail.presentation() {
                                            WorkPresentation::Work => WorkPresentation::Issue,
                                            WorkPresentation::Issue => WorkPresentation::Work,
                                        };
                                        this.set_presentation(next, true, cx);
                                    })),
                            )
                        })
                        .child(
                            Button::new("work-command-next-block", "→ · Next Block")
                                .style(ButtonStyle::Subtle)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.select_relative_block(1, cx);
                                })),
                        )
                        .child(
                            Button::new("work-command-close", "Esc · Close commands")
                                .style(ButtonStyle::Subtle)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.command_menu_open = false;
                                    cx.notify();
                                })),
                        ),
                )
            })
            .into_any_element()
    }

    fn render_blocks(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors();
        let selected_ref = self.detail.selected_block().map(|block| &block.block_ref);
        let buttons = self.detail.blocks().iter().map(|block| {
            let block_ref = block.block_ref.clone();
            let selected = selected_ref == Some(&block.block_ref);
            Button::new(
                format!("omega-work-block-{}", block.block_ref.0),
                block.kind.label(),
            )
            .style(if selected {
                ButtonStyle::Filled
            } else {
                ButtonStyle::Subtle
            })
            .size(ButtonSize::Compact)
            .disabled(!block.available)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select_block(block_ref.clone(), cx);
            }))
        });
        let selected = self.detail.selected_block();
        v_flex()
            .gap_2()
            .child(
                div()
                    .id("omega-work-blocks-heading")
                    .role(gpui::Role::Heading)
                    .aria_level(2)
                    .text_size(px(13.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(colors.text)
                    .child("Blocks"),
            )
            .child(h_flex().gap_1().flex_wrap().children(buttons))
            .when_some(selected, |section, block| {
                section.child(render_block_card(block, cx))
            })
            .into_any_element()
    }

    fn render_relations(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors();
        let relations = &self.detail.snapshot().relations;
        v_flex()
            .gap_2()
            .child(
                div()
                    .id("omega-work-relations-heading")
                    .role(gpui::Role::Heading)
                    .aria_level(2)
                    .text_size(px(13.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(colors.text)
                    .child(format!("Relations · {}", relations.len())),
            )
            .child(
                v_flex()
                    .id("omega-work-relations")
                    .rounded_lg()
                    .border_1()
                    .border_color(colors.border_variant)
                    .role(gpui::Role::List)
                    .when(relations.is_empty(), |list| {
                        list.child(
                            div()
                                .id("omega-work-relations-empty")
                                .p_3()
                                .role(gpui::Role::Status)
                                .text_size(px(12.))
                                .text_color(colors.text_muted)
                                .child("No source-supplied Work relations."),
                        )
                    })
                    .children(relations.iter().enumerate().map(|(index, relation)| {
                        h_flex()
                            .id(("omega-work-relation", index))
                            .min_h(px(38.))
                            .px_3()
                            .gap_3()
                            .border_b_1()
                            .border_color(colors.border_variant)
                            .role(gpui::Role::ListItem)
                            .aria_label(format!(
                                "{} {}",
                                work_contract_label(&relation.kind),
                                relation.target_work_ref.0
                            ))
                            .child(
                                Label::new(work_contract_label(&relation.kind))
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .truncate()
                                    .text_size(px(12.))
                                    .child(relation.target_work_ref.0.clone()),
                            )
                    })),
            )
            .into_any_element()
    }

    fn render_activity(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors();
        let count = self.history.rows.len();
        let handle = self.history_scroll.clone();
        v_flex()
            .min_h(px(260.))
            .flex_1()
            .gap_2()
            .child(
                h_flex()
                    .justify_between()
                    .child(
                        div()
                            .id("omega-work-activity-heading")
                            .role(gpui::Role::Heading)
                            .aria_level(2)
                            .text_size(px(13.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(colors.text)
                            .child(format!("Activity · {count}")),
                    )
                    .when(self.history.truncated, |header| {
                        header.child(
                            Label::new(format!("{} older rows omitted", self.history.omitted))
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        )
                    }),
            )
            .child(
                v_flex()
                    .id("omega-work-activity")
                    .min_h(px(220.))
                    .flex_1()
                    .rounded_lg()
                    .border_1()
                    .border_color(colors.border_variant)
                    .overflow_hidden()
                    .role(gpui::Role::List)
                    .when(count == 0, |list| {
                        list.child(
                            v_flex()
                                .id("omega-work-activity-empty")
                                .flex_1()
                                .items_center()
                                .justify_center()
                                .role(gpui::Role::Status)
                                .text_size(px(12.))
                                .text_color(colors.text_muted)
                                .child("No source activity has been supplied."),
                        )
                    })
                    .when(count > 0, |list| {
                        list.child(
                            uniform_list(
                                "omega-work-activity-rows",
                                count,
                                cx.processor(Self::render_history_rows),
                            )
                            .flex_grow_1()
                            .track_scroll(&handle),
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_inspector(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors();
        let snapshot = self.detail.snapshot();
        let summary = &snapshot.summary;
        let assignee = summary.assignee.0.as_ref().map_or_else(
            || "Unassigned".to_string(),
            |value| value.principal_ref.0.clone(),
        );
        let (delegate, grant, generation) = summary
            .agent_delegate
            .as_ref()
            .and_then(Option::as_ref)
            .map_or_else(
                || ("None".to_string(), "None".to_string(), "None".to_string()),
                |value| {
                    (
                        value.agent_ref.0.clone(),
                        value.delegation_grant_ref.0.clone(),
                        value.generation.0.to_string(),
                    )
                },
            );
        let portfolio = summary.portfolio.as_ref().and_then(Option::as_ref);
        let mutation_authority = self.detail.capability().map_or_else(
            || "Read-only projection".to_string(),
            |capability| {
                format!(
                    "Source-admitted · {}",
                    capability
                        .operations
                        .iter()
                        .map(work_contract_label)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            },
        );
        let capability_generation = self.detail.capability().map_or_else(
            || "None".to_string(),
            |value| value.generation.0.to_string(),
        );
        let cursor = summary
            .completeness
            .cursor
            .as_ref()
            .and_then(Option::as_ref)
            .map_or_else(|| "None".to_string(), |value| value.0.clone());
        let inspector_rows = vec![
            ("Owner", summary.owner_ref.0.clone()),
            ("Assignee · human", assignee),
            ("Agent delegate", delegate),
            ("Delegation grant", grant),
            ("Delegate generation", generation),
            (
                "Source authority",
                work_contract_label(&summary.source_authority.kind),
            ),
            (
                "Source reference",
                summary.source_authority.source_ref.0.clone(),
            ),
            (
                "Adapter",
                summary.source_authority.adapter_version.0.clone(),
            ),
            ("Effective mutation authority", mutation_authority),
            ("Capability generation", capability_generation),
            ("Revision", summary.revision.0.to_string()),
            ("Updated", summary.updated_at.0.clone()),
            ("Freshness", work_contract_label(&summary.freshness.state)),
            ("Observed", summary.freshness.observed_at.0.clone()),
            (
                "Completeness",
                work_contract_label(&summary.completeness.state),
            ),
            ("Resume cursor", cursor),
            (
                "Visibility",
                work_contract_label(&summary.redaction.privacy_class),
            ),
            ("Visibility policy", summary.redaction.policy_ref.0.clone()),
            ("Participants", "Not supplied by source".into()),
            ("Watchers", "Not supplied by source".into()),
            ("Subscribers", "Not supplied by source".into()),
            ("Nostr references", "Not supplied by source".into()),
        ];
        v_flex()
            .id("omega-work-inspector")
            .debug_selector(|| "omega.omega.work-detail.inspector".into())
            .w(px(320.))
            .h_full()
            .flex_none()
            .overflow_y_scroll()
            .border_l_1()
            .border_color(colors.border_variant)
            .bg(colors.surface_background)
            .p_4()
            .gap_4()
            .role(gpui::Role::Complementary)
            .aria_label("Work inspector")
            .child(
                div()
                    .id("omega-work-inspector-heading")
                    .role(gpui::Role::Heading)
                    .aria_level(2)
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("Inspector"),
            )
            .child(
                v_flex()
                    .rounded_lg()
                    .border_1()
                    .border_color(colors.border_variant)
                    .children(
                        inspector_rows
                            .into_iter()
                            .map(|(label, value)| inspector_row(label, value, cx)),
                    ),
            )
            .when_some(portfolio, |inspector, portfolio| {
                inspector.child(
                    v_flex()
                        .gap_2()
                        .child(section_heading("Domain context", cx))
                        .child(
                            v_flex()
                                .rounded_lg()
                                .border_1()
                                .border_color(colors.border_variant)
                                .child(inspector_row(
                                    "Organization",
                                    portfolio.organization_ref.0.clone(),
                                    cx,
                                ))
                                .when_some(
                                    portfolio.team_ref.as_ref().and_then(Option::as_ref),
                                    |rows, value| {
                                        rows.child(inspector_row("Team", value.0.clone(), cx))
                                    },
                                )
                                .when_some(
                                    portfolio.initiative_ref.as_ref().and_then(Option::as_ref),
                                    |rows, value| {
                                        rows.child(inspector_row("Initiative", value.0.clone(), cx))
                                    },
                                )
                                .when_some(
                                    portfolio.project_ref.as_ref().and_then(Option::as_ref),
                                    |rows, value| {
                                        rows.child(inspector_row("Project", value.0.clone(), cx))
                                    },
                                )
                                .when_some(
                                    portfolio.cycle_ref.as_ref().and_then(Option::as_ref),
                                    |rows, value| {
                                        rows.child(inspector_row("Cycle", value.0.clone(), cx))
                                    },
                                )
                                .when_some(
                                    portfolio
                                        .project_milestone_ref
                                        .as_ref()
                                        .and_then(Option::as_ref),
                                    |rows, value| {
                                        rows.child(inspector_row(
                                            "Project milestone",
                                            value.0.clone(),
                                            cx,
                                        ))
                                    },
                                ),
                        ),
                )
            })
            .child(
                v_flex()
                    .gap_2()
                    .child(section_heading("Sessions and runs", cx))
                    .child(reference_list("Thread", &snapshot.thread_refs, cx))
                    .child(reference_list("Session", &snapshot.session_refs, cx))
                    .child(reference_list(
                        "Agent session",
                        &snapshot.agent_session_refs,
                        cx,
                    ))
                    .child(reference_list("Run", &snapshot.run_refs, cx)),
            )
            .child(
                v_flex()
                    .gap_2()
                    .child(section_heading("Evidence and decisions", cx))
                    .child(reference_list("Receipts", &snapshot.receipt_refs, cx))
                    .child(reference_list("Evidence", &snapshot.evidence_refs, cx))
                    .child(reference_list(
                        "Verification",
                        &snapshot.verification_refs,
                        cx,
                    ))
                    .child(reference_list(
                        "Owner disposition",
                        &snapshot.owner_disposition_refs,
                        cx,
                    ))
                    .child(inspector_row("Release", "Not supplied".into(), cx))
                    .child(inspector_row("Settlement", "Not supplied".into(), cx))
                    .child(inspector_row("Public claim", "Not supplied".into(), cx)),
            )
            .into_any_element()
    }
}

impl Render for WorkDetailSurface {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut key_context = KeyContext::new_with_defaults();
        key_context.add("OmegaWorkDetail");
        let work_ref = self.detail.snapshot().summary.work_ref.0.clone();
        h_flex()
            .id("omega-work-detail-surface")
            .debug_selector(move || format!("omega.omega.work-detail.{work_ref}"))
            .key_context(key_context)
            .track_focus(&self.focus_handle)
            .size_full()
            .overflow_hidden()
            .role(gpui::Role::Main)
            .aria_label(format!(
                "{} detail for {}",
                match self.detail.presentation() {
                    WorkPresentation::Work => "Work",
                    WorkPresentation::Issue => "Issue projection",
                },
                self.detail.snapshot().summary.title.0
            ))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.modifiers.modified() {
                    return;
                }
                match event.keystroke.key.as_str() {
                    "escape" if this.editing_title => this.cancel_title_edit(window, cx),
                    "escape" if this.command_menu_open => {
                        this.command_menu_open = false;
                        cx.notify();
                    }
                    "enter" if this.editing_title => this.submit_title_edit(window, cx),
                    "e" if !this.editing_title => this.start_title_edit(window, cx),
                    "left" | "h" if !this.editing_title => this.select_relative_block(-1, cx),
                    "right" | "l" if !this.editing_title => this.select_relative_block(1, cx),
                    "o" if !this.editing_title => {
                        cx.emit(WorkDetailSurfaceEvent::OpenSource(this.item.clone()));
                    }
                    "i" if !this.editing_title && this.has_issue_projection() => {
                        let next = match this.detail.presentation() {
                            WorkPresentation::Work => WorkPresentation::Issue,
                            WorkPresentation::Issue => WorkPresentation::Work,
                        };
                        this.set_presentation(next, true, cx);
                    }
                    "c" | "/" if !this.editing_title => this.toggle_command_menu(cx),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .child(
                v_flex()
                    .id("omega-work-detail-scroll")
                    .min_w_0()
                    .h_full()
                    .flex_1()
                    .overflow_y_scroll()
                    .p_5()
                    .gap_5()
                    .child(self.render_header(cx))
                    .child(self.render_blocks(cx))
                    .child(self.render_relations(cx))
                    .child(self.render_activity(cx)),
            )
            .child(self.render_inspector(cx))
    }
}

impl Focusable for WorkDetailSurface {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<WorkDetailSurfaceEvent> for WorkDetailSurface {}

fn render_block_card(block: &WorkBlock, cx: &App) -> AnyElement {
    let colors = cx.theme().colors();
    v_flex()
        .id(format!("omega-work-block-content-{}", block.block_ref.0))
        .min_h(px(112.))
        .rounded_lg()
        .border_1()
        .border_color(colors.border_variant)
        .bg(colors.editor_background)
        .p_4()
        .gap_2()
        .role(gpui::Role::Group)
        .aria_label(format!("{} Block", block.kind.label()))
        .child(
            h_flex()
                .justify_between()
                .child(
                    Label::new(block.title.0.clone())
                        .size(LabelSize::Small)
                        .weight(gpui::FontWeight::SEMIBOLD),
                )
                .child(
                    Label::new(if block.available {
                        "Source-backed"
                    } else {
                        "Unavailable"
                    })
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
                ),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(colors.text_muted)
                .child(block.source_ref.0.clone()),
        )
        .child(
            div()
                .text_size(px(11.))
                .text_color(colors.text_placeholder)
                .child("This Block is a view over the named source. Visibility does not grant authority."),
        )
        .into_any_element()
}

fn field_chip(label: &str, value: String, cx: &App) -> AnyElement {
    h_flex()
        .px_2()
        .py_1()
        .gap_1()
        .rounded_md()
        .bg(cx.theme().colors().element_background)
        .child(
            Label::new(label.to_string())
                .size(LabelSize::XSmall)
                .color(Color::Muted),
        )
        .child(Label::new(value).size(LabelSize::XSmall))
        .into_any_element()
}

fn inspector_row(label: &str, value: String, cx: &App) -> AnyElement {
    v_flex()
        .px_3()
        .py_2()
        .gap_1()
        .border_b_1()
        .border_color(cx.theme().colors().border_variant)
        .child(
            Label::new(label.to_string())
                .size(LabelSize::XSmall)
                .color(Color::Muted),
        )
        .child(
            div()
                .text_size(px(11.))
                .text_color(cx.theme().colors().text)
                .child(value),
        )
        .into_any_element()
}

fn section_heading(label: &str, cx: &App) -> AnyElement {
    div()
        .id(format!("omega-work-section-heading-{label}"))
        .role(gpui::Role::Heading)
        .aria_level(3)
        .text_size(px(12.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(cx.theme().colors().text)
        .child(label.to_string())
        .into_any_element()
}

fn reference_list<T>(label: &str, refs: &[T], cx: &App) -> AnyElement
where
    T: ReferenceValue,
{
    inspector_row(
        label,
        if refs.is_empty() {
            "None".to_string()
        } else {
            refs.iter()
                .map(|value| value.reference_value())
                .collect::<Vec<_>>()
                .join(", ")
        },
        cx,
    )
}

trait ReferenceValue {
    fn reference_value(&self) -> String;
}

macro_rules! reference_value {
    ($($type:ty),+ $(,)?) => {
        $(
            impl ReferenceValue for $type {
                fn reference_value(&self) -> String {
                    self.0.clone()
                }
            }
        )+
    };
}

reference_value!(
    omega_effectd::all_work_contract::ThreadRef,
    omega_effectd::all_work_contract::SessionRef,
    omega_effectd::all_work_contract::AgentSessionRef,
    omega_effectd::all_work_contract::RunRef,
    omega_effectd::all_work_contract::ReceiptRef,
    omega_effectd::all_work_contract::EvidenceRef,
    omega_effectd::all_work_contract::VerificationRef,
    omega_effectd::all_work_contract::OwnerDispositionRef,
);

fn intent_outcome_label(outcome: &WorkIntentOutcome) -> String {
    match outcome {
        WorkIntentOutcome::Pending => "Pending · waiting for a canonical source Event.".into(),
        WorkIntentOutcome::Accepted { revision, .. } => {
            format!("Accepted · canonical revision {}.", revision.0)
        }
        WorkIntentOutcome::Rejected { detail, .. } => format!("Rejected · {}", detail.0),
        WorkIntentOutcome::Offline => {
            "Offline · the Intent was not submitted and confirmed state did not change.".into()
        }
        WorkIntentOutcome::Conflict { current_revision } => format!(
            "Conflict · source revision is {}. Refresh before retrying.",
            current_revision.0
        ),
        WorkIntentOutcome::StaleGeneration { current_generation } => format!(
            "Stale generation · source generation is {}.",
            current_generation.0
        ),
    }
}

fn work_contract_label<T: std::fmt::Debug>(value: &T) -> String {
    format!("{value:?}")
        .replace('_', " ")
        .split_whitespace()
        .map(|word| {
            let mut characters = word.chars();
            characters.next().map_or_else(String::new, |first| {
                first.to_uppercase().collect::<String>() + characters.as_str()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}
