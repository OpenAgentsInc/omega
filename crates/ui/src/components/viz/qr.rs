//! Canvas QR renderer for invoices and deposit addresses.

use documented::Documented;
use gpui::{PathBuilder, canvas, point, px};
use qrcode::{QrCode as EncodedQrCode, types::Color as QrModuleColor};

use crate::components::viz::MarketTokens;
use crate::prelude::*;

const QUIET_ZONE_MODULES: usize = 4;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QrMatrix {
    width: usize,
    modules: Vec<bool>,
}

impl QrMatrix {
    pub fn encode(payload: &[u8]) -> Result<Self, qrcode::types::QrError> {
        let code = EncodedQrCode::new(payload)?;
        Ok(Self {
            width: code.width(),
            modules: code
                .to_colors()
                .into_iter()
                .map(|module| module == QrModuleColor::Dark)
                .collect(),
        })
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn is_dark(&self, x: usize, y: usize) -> bool {
        if x >= self.width || y >= self.width {
            return false;
        }
        y.checked_mul(self.width)
            .and_then(|row| row.checked_add(x))
            .and_then(|index| self.modules.get(index))
            .copied()
            .unwrap_or(false)
    }
}

#[derive(IntoElement, RegisterComponent, Documented)]
/// A QR matrix painted directly on a GPUI canvas.
pub struct QrCodeCanvas {
    matrix: QrMatrix,
    size: f32,
    tokens: Option<MarketTokens>,
}

impl QrCodeCanvas {
    pub fn new(matrix: QrMatrix) -> Self {
        Self {
            matrix,
            size: 184.0,
            tokens: None,
        }
    }

    pub fn encode(payload: impl AsRef<[u8]>) -> Result<Self, qrcode::types::QrError> {
        Ok(Self::new(QrMatrix::encode(payload.as_ref())?))
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size.max(64.0);
        self
    }

    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}

impl RenderOnce for QrCodeCanvas {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let matrix = self.matrix;
        div()
            .debug_selector(|| "market.qr".into())
            .size(px(self.size))
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        let module_span = matrix.width() + QUIET_ZONE_MODULES * 2;
                        if module_span == 0 {
                            return;
                        }
                        let mut background = PathBuilder::fill();
                        let right = bounds.origin.x + bounds.size.width;
                        let bottom = bounds.origin.y + bounds.size.height;
                        background.move_to(bounds.origin);
                        background.line_to(point(right, bounds.origin.y));
                        background.line_to(point(right, bottom));
                        background.line_to(point(bounds.origin.x, bottom));
                        background.close();
                        if let Ok(path) = background.build() {
                            window.paint_path(path, tokens.surface);
                        }
                        let module_size = f32::from(bounds.size.width)
                            .min(f32::from(bounds.size.height))
                            / module_span as f32;
                        if module_size <= 0.0 {
                            return;
                        }
                        let mut builder = PathBuilder::fill();
                        for y in 0..matrix.width() {
                            for x in 0..matrix.width() {
                                if !matrix.is_dark(x, y) {
                                    continue;
                                }
                                let left = bounds.origin.x
                                    + px((x + QUIET_ZONE_MODULES) as f32 * module_size);
                                let top = bounds.origin.y
                                    + px((y + QUIET_ZONE_MODULES) as f32 * module_size);
                                let right = left + px(module_size);
                                let bottom = top + px(module_size);
                                builder.move_to(point(left, top));
                                builder.line_to(point(right, top));
                                builder.line_to(point(right, bottom));
                                builder.line_to(point(left, bottom));
                                builder.close();
                            }
                        }
                        if let Ok(path) = builder.build() {
                            window.paint_path(path, tokens.text);
                        }
                    },
                )
                .size_full(),
            )
    }
}

impl Component for QrCodeCanvas {
    fn scope() -> ComponentScope {
        ComponentScope::DataDisplay
    }

    fn description() -> &'static str {
        Self::DOCS
    }

    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        let matrix =
            QrMatrix::encode(b"lightning:lnbc2500n1pomega293").unwrap_or_else(|_| QrMatrix {
                width: 0,
                modules: Vec::new(),
            });
        v_flex()
            .gap_4()
            .child(example_group_with_title(
                "QR code",
                vec![single_example(
                    "Canvas modules with the required four-module quiet zone",
                    QrCodeCanvas::new(matrix.clone()).into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "High-contrast modules remain machine-readable without hue",
                    QrCodeCanvas::new(matrix)
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
    fn encoded_matrix_is_square_and_bounded() {
        let Ok(matrix) = QrMatrix::encode(b"lnbc2500n1pomega293") else {
            panic!("QR fixture must encode");
        };
        assert!(matrix.width() >= 21);
        assert_eq!(matrix.modules.len(), matrix.width() * matrix.width());
        assert!(!matrix.is_dark(matrix.width(), 0));
    }
}
