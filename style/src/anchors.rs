use std::collections::HashMap;

use cssimpler_core::{LayoutBox, RenderNode};
use lightningcss::{printer::PrinterOptions, properties::Property};

use crate::Declaration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnchorArea {
    Top,
    Bottom,
    Left,
    Right,
}

impl AnchorArea {
    fn parse(value: &str) -> Option<Self> {
        Some(match value.trim().to_ascii_lowercase().as_str() {
            "top" => Self::Top,
            "bottom" => Self::Bottom,
            "left" => Self::Left,
            "right" => Self::Right,
            _ => return None,
        })
    }

    fn place(self, anchor: LayoutBox, popup: LayoutBox) -> (f32, f32) {
        let centered_x = anchor.x + (anchor.width - popup.width) / 2.0;
        let centered_y = anchor.y + (anchor.height - popup.height) / 2.0;
        match self {
            Self::Top => (centered_x, anchor.y - popup.height),
            Self::Bottom => (centered_x, anchor.y + anchor.height),
            Self::Left => (anchor.x - popup.width, centered_y),
            Self::Right => (anchor.x + anchor.width, centered_y),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnchorFallback {
    Area(AnchorArea),
    Named(String),
}

pub(crate) fn extract(property: &Property<'_>) -> Option<Declaration> {
    let id = property.property_id();
    let name = id.name();
    let value = || property.value_to_css_string(PrinterOptions::default()).ok();
    match name {
        "anchor-name" => Some(Declaration::AnchorNames(parse_names(&value()?, ','))),
        "position-anchor" => Some(Declaration::PositionAnchor(
            parse_names(&value()?, ',').into_iter().next(),
        )),
        "position-area" => Some(Declaration::AnchorArea(AnchorArea::parse(&value()?))),
        "position-try-fallbacks" => {
            let value = value()?;
            let fallbacks = value
                .split(',')
                .filter_map(|item| {
                    AnchorArea::parse(item)
                        .map(AnchorFallback::Area)
                        .or_else(|| {
                            parse_names(item, ',')
                                .into_iter()
                                .next()
                                .map(AnchorFallback::Named)
                        })
                })
                .collect();
            Some(Declaration::AnchorFallbacks(fallbacks))
        }
        _ => None,
    }
}

fn parse_names(value: &str, separator: char) -> Vec<String> {
    use cssparser::{Parser, ParserInput};
    value
        .split(separator)
        .filter_map(|item| {
            let mut input = ParserInput::new(item.trim());
            Parser::new(&mut input)
                .parse_entirely(|input| {
                    let name = input.expect_ident_cloned()?;
                    Ok::<_, cssparser::ParseError<'_, ()>>(
                        name.starts_with("--").then(|| name.to_string()),
                    )
                })
                .ok()
                .flatten()
        })
        .collect()
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Placement {
    pub(crate) names: Vec<String>,
    pub(crate) target: Option<String>,
    pub(crate) area: Option<AnchorArea>,
    pub(crate) fallback_areas: Vec<AnchorArea>,
    pub(crate) fallbacks: Vec<AnchorFallback>,
}

pub(crate) fn choose_position(
    placement: &Placement,
    anchor: LayoutBox,
    popup: LayoutBox,
    containing: LayoutBox,
) -> Option<(f32, f32)> {
    let primary = placement.area?;
    let initial = primary.place(anchor, popup);
    let fits = |(x, y)| {
        x >= containing.x
            && y >= containing.y
            && x + popup.width <= containing.x + containing.width
            && y + popup.height <= containing.y + containing.height
    };
    std::iter::once(primary)
        .chain(placement.fallback_areas.iter().copied())
        .map(|area| area.place(anchor, popup))
        .find(|&position| fits(position))
        .or(Some(initial))
}

pub(crate) type Anchors = HashMap<String, LayoutBox>;

pub(crate) fn move_rect(node: &mut RenderNode, dx: f32, dy: f32) {
    node.layout.x += dx;
    node.layout.y += dy;
}

pub(crate) fn collect_position_tries(
    rules: &lightningcss::rules::CssRuleList<'_, impl Clone>,
) -> HashMap<String, AnchorArea> {
    use lightningcss::rules::CssRule;

    rules
        .0
        .iter()
        .filter_map(|rule| {
            let CssRule::PositionTry(rule) = rule else {
                return None;
            };
            let area = rule
                .declarations
                .iter()
                .filter_map(|(property, _)| match extract(property) {
                    Some(Declaration::AnchorArea(area)) => Some(area),
                    _ => None,
                })
                .last()
                .flatten()?;
            Some((rule.name.0.to_string(), area))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use cssimpler_core::{ElementNode, Node};

    #[test]
    fn named_fallback_repositions_popup_and_its_contents() {
        let sheet = crate::parse_stylesheet(
            ".root { position: relative; width: 200px; height: 100px }
             .button { position: absolute; width: 30px; height: 10px; left: 85px; top: 85px; anchor-name: --button }
             .popup { position: absolute; width: 70px; height: 20px; position-anchor: --button; position-area: bottom; position-try-fallbacks: --above }
             @position-try --above { position-area: top; }",
        ).unwrap();
        let root: Node = ElementNode::new("main")
            .with_class("root")
            .with_child(ElementNode::new("button").with_class("button").into())
            .with_child(
                ElementNode::new("panel")
                    .with_class("popup")
                    .with_child(
                        ElementNode::new("span")
                            .with_child(Node::text("menu"))
                            .into(),
                    )
                    .into(),
            )
            .into();
        let rendered = crate::build_render_tree(&root, &sheet);
        let popup = &rendered.children[1];
        assert_eq!((popup.layout.x, popup.layout.y), (65.0, 65.0));
        assert!(popup.children[0].layout.y >= popup.layout.y);
    }
}
