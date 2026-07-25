use std::sync::Arc;

use gpui::Transformation;
use gpui::{App, IntoElement, Rems, RenderOnce, Size, Styled, Window, svg};
use serde::{Deserialize, Serialize};
use strum::{EnumIter, EnumString, IntoStaticStr};

use crate::prelude::*;
use crate::traits::transformable::Transformable;

#[derive(
    Debug, PartialEq, Eq, Copy, Clone, EnumIter, EnumString, IntoStaticStr, Serialize, Deserialize,
)]
#[strum(serialize_all = "snake_case")]
pub enum VectorName {
    BusinessStamp,
    VipStamp,
    Grid,
    OmegaLogo,
    ProTrialStamp,
    ProUserStamp,
    StudentStamp,
}

impl VectorName {
    /// Returns the path to this vector image.
    pub fn path(&self) -> Arc<str> {
        let file_stem: &'static str = self.into();
        format!("images/{file_stem}.svg").into()
    }
}

/// A vector image, such as an SVG.
///
/// A [`Vector`] is different from an [`crate::Icon`] in that it is intended
/// to be displayed at a specific size, or series of sizes, rather
/// than conforming to the standard size of an icon.
#[derive(IntoElement, RegisterComponent)]
pub struct Vector {
    path: Arc<str>,
    color: Color,
    size: Size<Rems>,
    transformation: Transformation,
}

impl Vector {
    /// Creates a new [`Vector`] image with the given [`VectorName`] and size.
    pub fn new(vector: VectorName, width: Rems, height: Rems) -> Self {
        Self {
            path: vector.path(),
            color: Color::default(),
            size: Size { width, height },
            transformation: Transformation::default(),
        }
    }

    /// Creates a new [`Vector`] image where the width and height are the same.
    pub fn square(vector: VectorName, size: Rems) -> Self {
        Self::new(vector, size, size)
    }

    /// Sets the vector color.
    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    /// Sets the vector size.
    pub fn size(mut self, size: impl Into<Size<Rems>>) -> Self {
        let size = size.into();
        self.size = size;
        self
    }
}

impl Transformable for Vector {
    fn transform(mut self, transformation: Transformation) -> Self {
        self.transformation = transformation;
        self
    }
}

impl RenderOnce for Vector {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let width = self.size.width;
        let height = self.size.height;

        svg()
            // By default, prevent the SVG from stretching
            // to fill its container.
            .flex_none()
            .w(width)
            .h(height)
            .path(self.path)
            .text_color(self.color.color(cx))
            .with_transformation(self.transformation)
    }
}

impl Component for Vector {
    fn scope() -> ComponentScope {
        ComponentScope::Images
    }

    fn name() -> &'static str {
        "Vector"
    }

    fn description() -> &'static str {
        "A vector image component that can be displayed at specific sizes."
    }

    fn preview(_window: &mut Window, _cx: &mut App) -> AnyElement {
        let size = rems_from_px(60.);

        v_flex()
            .gap_6()
            .children(vec![
                example_group_with_title(
                    "Basic Usage",
                    vec![
                        single_example(
                            "Default",
                            Vector::square(VectorName::OmegaLogo, size).into_any_element(),
                        ),
                        single_example(
                            "Custom Size",
                            h_flex()
                                .h(rems_from_px(120.))
                                .justify_center()
                                .child(Vector::new(
                                    VectorName::OmegaLogo,
                                    rems_from_px(120.),
                                    rems_from_px(200.),
                                ))
                                .into_any_element(),
                        ),
                    ],
                ),
                example_group_with_title(
                    "Colored",
                    vec![
                        single_example(
                            "Accent Color",
                            Vector::square(VectorName::OmegaLogo, size)
                                .color(Color::Accent)
                                .into_any_element(),
                        ),
                        single_example(
                            "Error Color",
                            Vector::square(VectorName::OmegaLogo, size)
                                .color(Color::Error)
                                .into_any_element(),
                        ),
                    ],
                ),
                example_group_with_title(
                    "Different Vectors",
                    vec![single_example(
                        "Grid",
                        Vector::square(VectorName::Grid, rems_from_px(100.)).into_any_element(),
                    )],
                ),
            ])
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector_path() {
        assert_eq!(
            VectorName::OmegaLogo.path().as_ref(),
            "images/omega_logo.svg"
        );
        assert_eq!(VectorName::Grid.path().as_ref(), "images/grid.svg");
    }

    /// OMEGA-DELTA-0022. `assets/images/` was outside every brand gate, so
    /// `zed_logo.svg` and `zed_x_copilot.svg` were embedded in the signed
    /// `0.2.0-rc11` binary and the Zed `Z` rendered through this component in
    /// the release command palette. Neither the artwork nor a name to restore
    /// it under may come back.
    #[test]
    fn no_vector_name_carries_a_competitor_name() {
        use strum::IntoEnumIterator as _;

        for vector in VectorName::iter() {
            let path = vector.path();
            let name: &'static str = vector.into();
            assert!(
                !path.contains("zed"),
                "VectorName::{name:?} resolves to a competitor-named asset: {path}"
            );
        }
    }
}
