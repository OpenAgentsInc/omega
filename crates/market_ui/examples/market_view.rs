//! Wasm P0 renderer for Immortal's embedded MKT-SWP fixture corpus.
//!
//! This is a no-spend document. It performs no relay connection, signing,
//! wallet action, or venue effect.

#![cfg_attr(not(target_family = "wasm"), allow(dead_code, unused_imports))]

use gpui::prelude::*;
use gpui::{App, Bounds, Context, Font, Pixels, Window, WindowBounds, WindowOptions, px, size};
use immortal_client::mkt_swp_client::REQUESTER_SESSION_VIEW_SCHEMA;
use market_ui::{
    CustodyView, ExitPackageView, FeeBreakdownView, MARKET_SESSION_VIEW_SCHEMA,
    MarketSessionViewModel, OfferingSideView, OfferingView, PriceFeedView, ProviderView, QuoteView,
    ReservationProofClass, ReservationView, SwapFlow, TimelineLaneView, TimelineSlotState,
    TimelineSlotView, TypedErrorView, VerifyChecklistView, VerifyRowView, VerifyState, asset_view,
};
use serde::Deserialize;
use theme::{ThemeSettingsProvider, UiDensity};
use ui::v_flex;

const IMMORTAL_FIXTURE: &str = include_str!("../fixtures/swp-client-engine-v1.json");

#[derive(Deserialize)]
struct FixtureCorpus {
    schema: String,
    source: FixtureSource,
    deterministic_session: DeterministicSession,
    flows: Vec<FixtureFlow>,
    verify_before_fund: Vec<FixtureFailure>,
}

#[derive(Deserialize)]
struct FixtureSource {
    commit: String,
    issue: String,
}

#[derive(Deserialize)]
struct DeterministicSession {
    session_id: String,
    offering_id: String,
    chain_asset_a: String,
    lightning_asset_a: String,
    funding_amount: String,
    invoice_observed_at: u64,
}

#[derive(Deserialize)]
struct FixtureFlow {
    name: String,
    swap_type: String,
    terminal: String,
}

#[derive(Deserialize)]
struct FixtureFailure {
    error: String,
}

struct WebThemeSettings {
    ui_font: Font,
    buffer_font: Font,
}

impl ThemeSettingsProvider for WebThemeSettings {
    fn ui_font<'a>(&'a self, _cx: &'a App) -> &'a Font {
        &self.ui_font
    }

    fn buffer_font<'a>(&'a self, _cx: &'a App) -> &'a Font {
        &self.buffer_font
    }

    fn ui_font_size(&self, _cx: &App) -> Pixels {
        px(14.)
    }

    fn buffer_font_size(&self, _cx: &App) -> Pixels {
        px(13.)
    }

    fn ui_density(&self, _cx: &App) -> UiDensity {
        UiDensity::Default
    }
}

struct FixtureDemo {
    view: Result<MarketSessionViewModel, String>,
    now: u64,
}

impl FixtureDemo {
    fn new(_cx: &mut Context<Self>) -> Self {
        let view = fixture_view().and_then(|view| {
            view.validate()?;
            Ok(view)
        });
        Self {
            view,
            now: 1_674_164_660,
        }
    }
}

impl Render for FixtureDemo {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("market-fixture-demo")
            .size_full()
            .overflow_y_scroll()
            .p_4()
            .gap_2()
            .child("DEMO · no keys · no funds · no orders")
            .when_some(self.view.as_ref().ok().cloned(), |page, view| {
                page.child(SwapFlow::new(view, self.now))
            })
            .when_some(self.view.as_ref().err().cloned(), |page, error| {
                page.child(format!("fixture refused: {error}"))
            })
    }
}

fn fixture_view() -> Result<MarketSessionViewModel, String> {
    let replay = immortal_client::mkt_swp_client::fixture_replay::replay_embedded_manifest()
        .map_err(|error| format!("embedded fixture replay {}: {error}", error.code()))?;
    let fixture: FixtureCorpus =
        serde_json::from_str(IMMORTAL_FIXTURE).map_err(|error| format!("fixture JSON: {error}"))?;
    if fixture.schema != market_ui::MARKET_FIXTURE_CORPUS_SCHEMA {
        return Err("fixture schema is unsupported".to_owned());
    }
    let copied = immortal_client::mkt_swp_client::fixture_replay::replay_manifest_bytes(
        IMMORTAL_FIXTURE.as_bytes(),
    )
    .map_err(|error| format!("rendered fixture replay {}: {error}", error.code()))?;
    if copied != replay {
        return Err("rendered fixture differs from the pin-embedded corpus".to_owned());
    }
    let session = fixture.deterministic_session;
    let provider_id = "11".repeat(32);
    let flow_labels = fixture
        .flows
        .iter()
        .map(|flow| format!("{} · {} · {}", flow.name, flow.swap_type, flow.terminal))
        .collect::<Vec<_>>();
    let typed_errors = fixture
        .verify_before_fund
        .iter()
        .take(3)
        .map(|failure| TypedErrorView {
            code: failure.error.clone(),
        })
        .collect();
    let source_assertion = market_ui::ProviderAssertionView {
        assertion: format!(
            "{} cases · {} custody tripwires · {}",
            replay.cases, replay.custody_tripwires, fixture.source.issue
        ),
        asserter: fixture.source.commit,
    };
    let input = asset_view(&session.chain_asset_a);
    let output = asset_view(&session.lightning_asset_a);
    Ok(MarketSessionViewModel {
        schema: MARKET_SESSION_VIEW_SCHEMA.to_owned(),
        engine_schema: REQUESTER_SESSION_VIEW_SCHEMA.to_owned(),
        session_id: session.session_id,
        phase: "fixture replay verified · no-spend".to_owned(),
        provider: ProviderView {
            provider_id: provider_id.clone(),
            display_name: "Immortal fixture provider".to_owned(),
            status: "fixture".to_owned(),
            profiles: vec!["mkt-swp:1".to_owned()],
            assertions: vec![source_assertion],
        },
        offering: Some(OfferingView {
            offering_id: session.offering_id,
            provider_id: provider_id.clone(),
            status: "fixture".to_owned(),
            profile: "mkt-swp".to_owned(),
            version: 1,
            published_at: session.invoice_observed_at,
            sides: vec![OfferingSideView {
                input: input.clone(),
                output: output.clone(),
                direction: "submarine".to_owned(),
                minimum_amount: "10000".to_owned(),
                maximum_amount: "1000000".to_owned(),
            }],
        }),
        quotes: vec![QuoteView {
            quote_id: "22".repeat(32),
            provider_id,
            quote_class: "firm".to_owned(),
            input,
            output,
            input_amount: session.funding_amount,
            output_amount: "99000".to_owned(),
            expires_at: 1_674_165_000,
            reservation: ReservationView {
                class: "soft".to_owned(),
                proof_class: ReservationProofClass::ProviderSigned,
            },
            fees: FeeBreakdownView {
                provider_fee: "500".to_owned(),
                miner_fee_budget: "500".to_owned(),
                lightning_routing_fee_budget: "0".to_owned(),
                fee_payer: "requester".to_owned(),
                rounding_rule: "floor_output_sats".to_owned(),
                amount_equation: "input_minus_provider_and_quoted_fees".to_owned(),
                maximum_total_fee: "1000".to_owned(),
            },
            price_feed: Some(PriceFeedView {
                url: "https://fixture.invalid/price".to_owned(),
                value_pointer: "/data/price".to_owned(),
                observed_value: "6543210".to_owned(),
                observed_at: session.invoice_observed_at,
                max_age_seconds: 300,
                response_sha256: "33".repeat(32),
            }),
            custody: CustodyView {
                funds_control: "no funds exist in this fixture".to_owned(),
                key_control: "no keys exist in this fixture".to_owned(),
                recovery_control: "fixture exit-package commitment".to_owned(),
                counterparty_exposure: "provider-signed reservation claim".to_owned(),
                maximum_custody_duration_seconds: 0,
                exact_height_bound: None,
                credential_exposure: "none".to_owned(),
            },
        }],
        timeline: vec![TimelineLaneView {
            signer_role: "fixture corpus".to_owned(),
            signer_pubkey: "none".to_owned(),
            slots: vec![TimelineSlotView {
                sequence: None,
                state: TimelineSlotState::Event,
                labels: flow_labels,
                event_ids: Vec::new(),
            }],
        }],
        verification: VerifyChecklistView {
            rows: [
                ("lock_script_tree", "Lock script and tree"),
                ("amounts", "Signed amounts"),
                ("payment_hash", "Payment hash"),
                ("timelocks", "Timelock ladder"),
                ("claim_path", "Claim path"),
                ("refund_path", "Refund path"),
            ]
            .into_iter()
            .map(|(check_id, label)| VerifyRowView {
                check_id: check_id.to_owned(),
                label: label.to_owned(),
                state: VerifyState::Passed,
                error_code: None,
            })
            .collect(),
            engine_funding_authorized: false,
        },
        exit_package: ExitPackageView {
            exists: true,
            artifact_sha256: Some("44".repeat(32)),
            latest_safe_height: Some(200),
        },
        receipt: None,
        errors: typed_errors,
    })
}

#[cfg(target_family = "wasm")]
fn main() {
    gpui_platform::web_init();
    let handle = gpui_platform::application().run_embedded(|cx: &mut App| {
        theme::set_theme_settings_provider(
            Box::new(WebThemeSettings {
                ui_font: gpui::font("IBM Plex Sans"),
                buffer_font: gpui::font("Lilex"),
            }),
            cx,
        );
        theme::init(theme::LoadThemes::JustBase, cx);
        let bounds = Bounds::centered(None, size(px(1100.), px(800.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(FixtureDemo::new),
        )
        .expect("failed to open fixture window");
        cx.activate(true);
    });
    std::mem::forget(handle);
}

#[cfg(not(target_family = "wasm"))]
fn main() {}
