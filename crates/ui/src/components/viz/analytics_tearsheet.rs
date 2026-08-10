//! Native, exportable composition of engine and ledger analytics renderers.

use documented::Documented;

use crate::components::viz::{
    AnalyticsLine, DemoEquitySeriesSource, DemoReturnHistogramSource, DemoStatisticGridSource,
    EquityCurve, EquitySeries, EquitySeriesSource, LineChart, LinePoint, LineSeries, MarketTokens,
    ReturnHistogramSource, ReturnSeries, ReturnsHistogram, StatisticGridSource, StatisticTileGrid,
    StatisticValue, format_with_decimals,
};
use crate::prelude::*;

#[derive(Clone, Debug, PartialEq)]
pub struct TearsheetData {
    pub title: SharedString,
    pub generated_at_ms: i64,
    pub equity: EquitySeries,
    pub rolling_sharpe: AnalyticsLine,
    pub pnl_distribution: ReturnSeries,
    pub statistics: Vec<StatisticValue>,
}

impl TearsheetData {
    pub fn export_csv(&self) -> String {
        let mut output = String::from("section,name,time_ms,value\n");
        for point in self.equity.points() {
            output.push_str(&format!(
                "equity,equity,{},{}\n",
                point.time_ms, point.equity_cents
            ));
        }
        for point in &self.rolling_sharpe.points {
            output.push_str(&format!(
                "rolling,sharpe,{},{}\n",
                point.time_ms,
                format_with_decimals(point.value, 6)
            ));
        }
        for (index, value) in self.pnl_distribution.values().iter().enumerate() {
            output.push_str(&format!(
                "distribution,pnl,{},{}\n",
                index,
                format_with_decimals(*value, 6)
            ));
        }
        for value in &self.statistics {
            output.push_str(&format!(
                "statistic,{},{},{}\n",
                value.kind.label(),
                self.generated_at_ms,
                format_with_decimals(value.value, 6)
            ));
        }
        output
    }
}

pub trait TearsheetSource {
    fn tearsheet(&self) -> TearsheetData;
}

pub struct DemoTearsheetSource;

impl TearsheetSource for DemoTearsheetSource {
    fn tearsheet(&self) -> TearsheetData {
        let start = 1_754_700_000_000i64;
        TearsheetData {
            title: "Momentum · BTC testnet".into(),
            generated_at_ms: start + 120 * 3_600_000,
            equity: DemoEquitySeriesSource.equity_series(),
            rolling_sharpe: AnalyticsLine {
                label: "Rolling Sharpe".into(),
                points: (0..120)
                    .map(|index| crate::components::viz::AnalyticsPoint {
                        time_ms: start + index * 3_600_000,
                        value: 1.2 + (index as f64 / 13.0).sin() * 0.7,
                    })
                    .collect(),
            },
            pnl_distribution: DemoReturnHistogramSource.return_series(),
            statistics: DemoStatisticGridSource.statistics(),
        }
    }
}

#[derive(IntoElement, RegisterComponent, Documented)]
/// Equity, drawdown, rolling Sharpe, PnL distribution, and statistics as one exportable report.
pub struct Tearsheet {
    data: TearsheetData,
    width: f32,
    tokens: Option<MarketTokens>,
}

impl Tearsheet {
    pub fn new(data: TearsheetData) -> Self {
        Self {
            data,
            width: 760.0,
            tokens: None,
        }
    }

    pub fn from_source(source: &impl TearsheetSource) -> Self {
        Self::new(source.tearsheet())
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = width.max(360.0);
        self
    }

    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }

    pub fn export_csv(&self) -> String {
        self.data.export_csv()
    }
}

impl RenderOnce for Tearsheet {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let rolling = LineSeries::new(
            self.data
                .rolling_sharpe
                .points
                .iter()
                .map(|point| LinePoint {
                    time_ms: point.time_ms,
                    value: point.value,
                })
                .collect(),
            2,
        );
        let chart_width = (self.width - 16.0).max(320.0);
        v_flex()
            .debug_selector(|| "market.tearsheet".into())
            .w(px(self.width))
            .p_3()
            .gap_3()
            .border_1()
            .border_color(tokens.grid)
            .bg(tokens.surface)
            .child(
                h_flex()
                    .justify_between()
                    .child(Label::new(self.data.title).size(LabelSize::Large))
                    .child(
                        Label::new(format!("export · {}", self.data.generated_at_ms))
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    ),
            )
            .child(
                EquityCurve::new(self.data.equity)
                    .size(chart_width, 270.0)
                    .tokens(tokens),
            )
            .child(
                h_flex()
                    .items_start()
                    .gap_2()
                    .child(
                        v_flex()
                            .child(Label::new("Rolling Sharpe").size(LabelSize::Small))
                            .child(
                                LineChart::new(rolling)
                                    .size(chart_width * 0.5, 180.0)
                                    .tokens(tokens),
                            ),
                    )
                    .child(
                        v_flex()
                            .child(Label::new("PnL distribution").size(LabelSize::Small))
                            .child(
                                ReturnsHistogram::new(self.data.pnl_distribution)
                                    .size(chart_width * 0.5, 180.0)
                                    .tokens(tokens),
                            ),
                    ),
            )
            .child(StatisticTileGrid::new(self.data.statistics).tokens(tokens))
    }
}

impl Component for Tearsheet {
    fn scope() -> ComponentScope {
        ComponentScope::DataDisplay
    }
    fn description() -> &'static str {
        Self::DOCS
    }
    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        v_flex()
            .gap_4()
            .child(example_group_with_title(
                "Native tearsheet",
                vec![single_example(
                    "One exportable strategy report",
                    Tearsheet::from_source(&DemoTearsheetSource).into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Pane structure, signs, and geometry retain every report section",
                    Tearsheet::from_source(&DemoTearsheetSource)
                        .tokens(MarketTokens::from_theme(cx).grayscale())
                        .into_any_element(),
                )],
            ))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_contains_every_report_section_without_recomputing_statistics() {
        let data = DemoTearsheetSource.tearsheet();
        let export = data.export_csv();
        assert!(export.contains("equity,equity"));
        assert!(export.contains("rolling,sharpe"));
        assert!(export.contains("distribution,pnl"));
        assert!(export.contains("statistic,Sharpe"));
        assert_eq!(data.statistics.len(), 13);
    }
}
