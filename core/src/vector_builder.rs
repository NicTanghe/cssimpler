//! Fluent construction for validated [`VectorPath`] values.
//!
//! [`VectorPathBuilder`] owns exactly one [`Vec<PathCommand>`] and one
//! [`VectorPaint`]. Chained methods are consuming (`self -> Self`) so the buffer
//! is moved, not copied, between calls. [`VectorPathBuilder::build`] delegates to
//! [`VectorPath::new`], so the resulting path is bit-for-bit the same value that
//! direct construction produces, and it is validated by the same rules.
use crate::{Color, PathCommand, VectorError, VectorPaint, VectorPath, VectorPoint};

/// A fluent, incrementally filled [`VectorPath`].
///
/// The builder starts with [`VectorPaint::default`]: black fill, no stroke, and a
/// stroke width of `1.0`. Coordinates and paints are checked when
/// [`build`](Self::build) is called, so authoring methods never panic on ordinary
/// malformed input.
///
/// ```rust
/// use cssimpler_core::{Color, VectorPath, VectorPoint as P};
///
/// let path = VectorPath::builder()
///     .reserve(4)
///     .move_to(P::new(0.0, 0.0))
///     .line_to(P::new(20.0, 0.0))
///     .quad_to(P::new(30.0, 10.0), P::new(20.0, 20.0))
///     .close()
///     .stroke(Color::rgb(20, 40, 60))
///     .stroke_width(2.0)
///     .build()
///     .unwrap();
///
/// assert_eq!(path.paint().fill, Some(Color::BLACK));
/// assert_eq!(path.commands().len(), 4);
/// ```
#[derive(Clone, Debug, PartialEq)]
#[must_use = "a builder does nothing unless it is finished with `build`"]
pub struct VectorPathBuilder {
    commands: Vec<PathCommand>,
    paint: VectorPaint,
}

impl Default for VectorPathBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl VectorPathBuilder {
    /// Start an empty builder with default paint.
    pub fn new() -> Self {
        Self {
            commands: Vec::new(),
            paint: VectorPaint::default(),
        }
    }

    /// Start an empty builder whose command buffer reserves space for `capacity`
    /// commands, avoiding reallocations while a known-length path is chained.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            commands: Vec::with_capacity(capacity),
            paint: VectorPaint::default(),
        }
    }

    /// Reserve room for at least `additional` more commands and return the
    /// builder, for use in a fluent chain. This has `Vec::reserve` semantics.
    pub fn reserve(mut self, additional: usize) -> Self {
        self.commands.reserve(additional);
        self
    }

    /// The commands added so far, in authoring order.
    pub fn commands(&self) -> &[PathCommand] {
        &self.commands
    }

    /// The paint that [`build`](Self::build) will validate and attach.
    pub fn paint(&self) -> VectorPaint {
        self.paint
    }

    /// Number of commands added so far.
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    /// Whether no commands have been added yet.
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Current command buffer capacity.
    pub fn capacity(&self) -> usize {
        self.commands.capacity()
    }

    /// Append a `MoveTo` command, starting a new contour.
    pub fn move_to(mut self, to: VectorPoint) -> Self {
        self.commands.push(PathCommand::MoveTo(to));
        self
    }

    /// Append a `LineTo` command.
    pub fn line_to(mut self, to: VectorPoint) -> Self {
        self.commands.push(PathCommand::LineTo(to));
        self
    }

    /// Append a `QuadTo` command with one control point and an endpoint.
    pub fn quad_to(mut self, control: VectorPoint, end: VectorPoint) -> Self {
        self.commands.push(PathCommand::QuadTo { control, end });
        self
    }

    /// Append a `CubicTo` command with two control points and an endpoint.
    pub fn cubic_to(
        mut self,
        control1: VectorPoint,
        control2: VectorPoint,
        end: VectorPoint,
    ) -> Self {
        self.commands.push(PathCommand::CubicTo {
            control1,
            control2,
            end,
        });
        self
    }

    /// Append a `Close` command, closing the current contour.
    pub fn close(mut self) -> Self {
        self.commands.push(PathCommand::Close);
        self
    }

    /// Set a solid fill color. This is the default fill (black) unless replaced
    /// or disabled.
    pub fn fill(mut self, color: Color) -> Self {
        self.paint.fill = Some(color);
        self
    }

    /// Disable the fill. The stroke is unaffected.
    pub fn no_fill(mut self) -> Self {
        self.paint.fill = None;
        self
    }

    /// Set a solid stroke color. Strokes are disabled by default.
    pub fn stroke(mut self, color: Color) -> Self {
        self.paint.stroke = Some(color);
        self
    }

    /// Disable the stroke. The fill is unaffected.
    pub fn no_stroke(mut self) -> Self {
        self.paint.stroke = None;
        self
    }

    /// Set the stroke width. Zero is accepted; negative and non-finite widths
    /// are rejected by [`build`](Self::build).
    pub fn stroke_width(mut self, width: f32) -> Self {
        self.paint.stroke_width = width;
        self
    }

    /// Replace the whole paint value, for example to reuse the paint of an
    /// existing [`VectorPath`].
    pub fn with_paint(mut self, paint: VectorPaint) -> Self {
        self.paint = paint;
        self
    }

    /// Validate the commands and paint and produce the [`VectorPath`].
    ///
    /// The command buffer is moved into the path; it is not copied or converted to
    /// a second geometry representation. Validation matches [`VectorPath::new`]
    /// exactly: coordinates must be finite, the path must start with a `MoveTo`,
    /// and the stroke width must be finite and nonnegative.
    pub fn build(self) -> Result<VectorPath, VectorError> {
        VectorPath::new(self.commands, self.paint)
    }
}

impl VectorPath {
    /// Start a fluent builder that produces the same validated path as
    /// [`VectorPath::new`].
    pub fn builder() -> VectorPathBuilder {
        VectorPathBuilder::new()
    }
}
