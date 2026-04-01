use crate::Color;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OverflowMode {
    #[default]
    Visible,
    Clip,
    Hidden,
    Auto,
    Scroll,
}

impl OverflowMode {
    pub const fn clips(self) -> bool {
        !matches!(self, Self::Visible)
    }

    pub const fn allows_scrolling(self) -> bool {
        matches!(self, Self::Auto | Self::Scroll)
    }

    pub const fn reserves_gutter(self) -> bool {
        matches!(self, Self::Scroll)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScrollbarWidth {
    Auto,
    Thin,
    None,
    Px(f32),
}

impl Default for ScrollbarWidth {
    fn default() -> Self {
        Self::Auto
    }
}

impl ScrollbarWidth {
    pub fn resolve_px(self) -> f32 {
        match self {
            Self::Auto => 12.0,
            Self::Thin => 8.0,
            Self::None => 0.0,
            Self::Px(value) => value.max(0.0),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollbarStyle {
    pub width: ScrollbarWidth,
    pub thumb_color: Option<Color>,
    pub track_color: Option<Color>,
}

impl ScrollbarStyle {
    pub fn resolved_width(self) -> f32 {
        self.width.resolve_px()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScrollbarAxisState {
    pub track_hovered: bool,
    pub thumb_hovered: bool,
    pub thumb_active: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScrollbarInteractionState {
    pub horizontal: ScrollbarAxisState,
    pub vertical: ScrollbarAxisState,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollbarMetrics {
    pub offset_x: f32,
    pub offset_y: f32,
    pub max_offset_x: f32,
    pub max_offset_y: f32,
    pub reserved_width: f32,
    pub reserved_height: f32,
}

impl ScrollbarMetrics {
    pub fn clamp_offsets(&mut self) {
        self.offset_x = self.offset_x.clamp(0.0, self.max_offset_x.max(0.0));
        self.offset_y = self.offset_y.clamp(0.0, self.max_offset_y.max(0.0));
    }

    pub fn can_scroll_x(self) -> bool {
        self.max_offset_x > 0.0
    }

    pub fn can_scroll_y(self) -> bool {
        self.max_offset_y > 0.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollbarData {
    pub overflow_x: OverflowMode,
    pub overflow_y: OverflowMode,
    pub style: ScrollbarStyle,
    pub metrics: ScrollbarMetrics,
    pub interaction: ScrollbarInteractionState,
}

impl ScrollbarData {
    pub fn new(
        overflow_x: OverflowMode,
        overflow_y: OverflowMode,
        style: ScrollbarStyle,
        metrics: ScrollbarMetrics,
    ) -> Self {
        let mut data = Self {
            overflow_x,
            overflow_y,
            style,
            metrics,
            interaction: ScrollbarInteractionState::default(),
        };
        data.metrics.clamp_offsets();
        data
    }

    pub fn resolved_width(self) -> f32 {
        self.style.resolved_width()
    }

    pub fn clamp_offsets(&mut self) {
        self.metrics.clamp_offsets();
    }

    pub fn shows_horizontal(self) -> bool {
        self.resolved_width() > 0.0
            && match self.overflow_x {
                OverflowMode::Scroll => true,
                OverflowMode::Auto => self.metrics.can_scroll_x(),
                _ => false,
            }
    }

    pub fn shows_vertical(self) -> bool {
        self.resolved_width() > 0.0
            && match self.overflow_y {
                OverflowMode::Scroll => true,
                OverflowMode::Auto => self.metrics.can_scroll_y(),
                _ => false,
            }
    }

    pub fn needs_translation(self) -> bool {
        self.metrics.offset_x.abs() > f32::EPSILON || self.metrics.offset_y.abs() > f32::EPSILON
    }
}
