use cssimpler_core::{LayoutBox, Transform2D, TransformOperation};

use super::ClipRect;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct AffineTransform {
    pub(crate) a: f32,
    pub(crate) b: f32,
    pub(crate) c: f32,
    pub(crate) d: f32,
    pub(crate) e: f32,
    pub(crate) f: f32,
}

impl AffineTransform {
    pub(crate) const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    pub(crate) fn is_identity(self) -> bool {
        self == Self::IDENTITY
    }

    pub(crate) fn translate(x: f32, y: f32) -> Self {
        Self {
            e: x,
            f: y,
            ..Self::IDENTITY
        }
    }

    pub(crate) fn scale(x: f32, y: f32) -> Self {
        Self {
            a: x,
            d: y,
            ..Self::IDENTITY
        }
    }

    pub(crate) fn rotate_degrees(degrees: f32) -> Self {
        let radians = degrees.to_radians();
        let cos = radians.cos();
        let sin = radians.sin();
        Self {
            a: cos,
            b: sin,
            c: -sin,
            d: cos,
            e: 0.0,
            f: 0.0,
        }
    }

    pub(crate) fn multiply(self, other: Self) -> Self {
        Self {
            a: self.a * other.a + self.c * other.b,
            b: self.b * other.a + self.d * other.b,
            c: self.a * other.c + self.c * other.d,
            d: self.b * other.c + self.d * other.d,
            e: self.a * other.e + self.c * other.f + self.e,
            f: self.b * other.e + self.d * other.f + self.f,
        }
    }

    pub(crate) fn transform_point(self, x: f32, y: f32) -> (f32, f32) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }

    pub(crate) fn invert(self) -> Option<Self> {
        let determinant = self.a * self.d - self.b * self.c;
        if determinant.abs() <= f32::EPSILON {
            return None;
        }
        let inverse = 1.0 / determinant;
        Some(Self {
            a: self.d * inverse,
            b: -self.b * inverse,
            c: -self.c * inverse,
            d: self.a * inverse,
            e: (self.c * self.f - self.d * self.e) * inverse,
            f: (self.b * self.e - self.a * self.f) * inverse,
        })
    }
}

#[derive(Clone, Debug)]
struct ClipRegion {
    layout: LayoutBox,
    inverse: AffineTransform,
}

#[derive(Clone, Debug)]
pub(crate) struct ClipState {
    pub(crate) coarse: ClipRect,
    regions: Vec<ClipRegion>,
}

impl ClipState {
    pub(crate) fn new(coarse: ClipRect) -> Self {
        Self {
            coarse,
            regions: Vec::new(),
        }
    }

    pub(crate) fn contains(&self, x: f32, y: f32) -> bool {
        self.coarse.contains(x, y)
            && self.regions.iter().all(|region| {
                let (local_x, local_y) = region.inverse.transform_point(x, y);
                layout_contains(region.layout, local_x, local_y)
            })
    }

    pub(crate) fn push_layout_clip(
        &self,
        layout: LayoutBox,
        matrix: AffineTransform,
    ) -> Option<Self> {
        let inverse = matrix.invert()?;
        let coarse = self
            .coarse
            .intersect(transform_layout_bounds(layout, matrix)?)?;
        let mut regions = self.regions.clone();
        regions.push(ClipRegion { layout, inverse });
        Some(Self { coarse, regions })
    }
}

pub(crate) fn node_transform_matrix(
    layout: LayoutBox,
    transform: &Transform2D,
) -> Option<AffineTransform> {
    if transform.is_identity() {
        return Some(AffineTransform::IDENTITY);
    }

    let origin_x = layout.x + transform.origin.x.resolve(layout.width);
    let origin_y = layout.y + transform.origin.y.resolve(layout.height);
    let mut matrix = AffineTransform::IDENTITY;
    for operation in &transform.operations {
        let operation = match operation {
            TransformOperation::Translate { x, y } => {
                AffineTransform::translate(x.resolve(layout.width), y.resolve(layout.height))
            }
            TransformOperation::Scale { x, y } => AffineTransform::scale(*x, *y),
            TransformOperation::Rotate { degrees } => AffineTransform::rotate_degrees(*degrees),
        };
        matrix = operation.multiply(matrix);
    }

    Some(
        AffineTransform::translate(origin_x, origin_y)
            .multiply(matrix)
            .multiply(AffineTransform::translate(-origin_x, -origin_y)),
    )
}

pub(crate) fn transform_layout_bounds(
    layout: LayoutBox,
    matrix: AffineTransform,
) -> Option<ClipRect> {
    transform_clip_rect(
        ClipRect {
            x0: layout.x,
            y0: layout.y,
            x1: layout.x + layout.width,
            y1: layout.y + layout.height,
        },
        matrix,
    )
}

pub(crate) fn transform_clip_rect(rect: ClipRect, matrix: AffineTransform) -> Option<ClipRect> {
    if rect.is_empty() {
        return None;
    }

    let corners = [
        matrix.transform_point(rect.x0, rect.y0),
        matrix.transform_point(rect.x1, rect.y0),
        matrix.transform_point(rect.x0, rect.y1),
        matrix.transform_point(rect.x1, rect.y1),
    ];
    let mut x0 = f32::INFINITY;
    let mut y0 = f32::INFINITY;
    let mut x1 = f32::NEG_INFINITY;
    let mut y1 = f32::NEG_INFINITY;
    for (x, y) in corners {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }

    let bounds = ClipRect { x0, y0, x1, y1 };
    (!bounds.is_empty()).then_some(bounds)
}

fn layout_contains(layout: LayoutBox, x: f32, y: f32) -> bool {
    x >= layout.x && y >= layout.y && x < layout.x + layout.width && y < layout.y + layout.height
}
