use cssimpler_core::{LayoutBox, Transform2D, TransformMatrix3d, TransformOperation};

use super::ClipRect;

const TRANSFORM_EPSILON: f32 = 1e-5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PerspectiveContext {
    pub(crate) depth: f32,
    pub(crate) origin_x: f32,
    pub(crate) origin_y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct AffineTransform {
    pub(crate) a: f32,
    pub(crate) b: f32,
    pub(crate) c: f32,
    pub(crate) d: f32,
    pub(crate) e: f32,
    pub(crate) f: f32,
    pub(crate) g: f32,
    pub(crate) h: f32,
    pub(crate) i: f32,
}

impl AffineTransform {
    pub(crate) const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
        g: 0.0,
        h: 0.0,
        i: 1.0,
    };

    pub(crate) const fn translate(x: f32, y: f32) -> Self {
        Self {
            e: x,
            f: y,
            ..Self::IDENTITY
        }
    }

    pub(crate) fn is_identity(self) -> bool {
        self == Self::IDENTITY
    }

    pub(crate) fn multiply(self, other: Self) -> Self {
        Self {
            a: self.a * other.a + self.c * other.b + self.e * other.g,
            b: self.b * other.a + self.d * other.b + self.f * other.g,
            c: self.a * other.c + self.c * other.d + self.e * other.h,
            d: self.b * other.c + self.d * other.d + self.f * other.h,
            e: self.a * other.e + self.c * other.f + self.e * other.i,
            f: self.b * other.e + self.d * other.f + self.f * other.i,
            g: self.g * other.a + self.h * other.b + self.i * other.g,
            h: self.g * other.c + self.h * other.d + self.i * other.h,
            i: self.g * other.e + self.h * other.f + self.i * other.i,
        }
    }

    pub(crate) fn transform_point(self, x: f32, y: f32) -> (f32, f32) {
        let denominator = self.g * x + self.h * y + self.i;
        if denominator.abs() <= TRANSFORM_EPSILON {
            return (f32::INFINITY, f32::INFINITY);
        }

        (
            (self.a * x + self.c * y + self.e) / denominator,
            (self.b * x + self.d * y + self.f) / denominator,
        )
    }

    pub(crate) fn invert(self) -> Option<Self> {
        let determinant = self.a * (self.d * self.i - self.f * self.h)
            - self.c * (self.b * self.i - self.f * self.g)
            + self.e * (self.b * self.h - self.d * self.g);
        if determinant.abs() <= TRANSFORM_EPSILON {
            return None;
        }

        let inverse = 1.0 / determinant;
        Some(Self {
            a: (self.d * self.i - self.f * self.h) * inverse,
            b: (self.f * self.g - self.b * self.i) * inverse,
            c: (self.e * self.h - self.c * self.i) * inverse,
            d: (self.a * self.i - self.e * self.g) * inverse,
            e: (self.c * self.f - self.e * self.d) * inverse,
            f: (self.b * self.e - self.a * self.f) * inverse,
            g: (self.b * self.h - self.d * self.g) * inverse,
            h: (self.c * self.g - self.a * self.h) * inverse,
            i: (self.a * self.d - self.b * self.c) * inverse,
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
                local_x.is_finite()
                    && local_y.is_finite()
                    && layout_contains(region.layout, local_x, local_y)
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

pub(crate) fn perspective_context(
    layout: LayoutBox,
    perspective: Option<f32>,
) -> Option<PerspectiveContext> {
    let depth = perspective?;
    (depth > TRANSFORM_EPSILON).then_some(PerspectiveContext {
        depth,
        origin_x: layout.x + layout.width * 0.5,
        origin_y: layout.y + layout.height * 0.5,
    })
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn node_transform_matrix(
    layout: LayoutBox,
    transform: &Transform2D,
    perspective: Option<PerspectiveContext>,
) -> Option<AffineTransform> {
    if transform.is_identity() {
        return Some(AffineTransform::IDENTITY);
    }

    if !transform.uses_depth() && perspective.is_none() {
        return Some(node_affine_transform_matrix(layout, transform));
    }

    project_world_transform_matrix(
        layout,
        node_local_transform_matrix(layout, transform),
        perspective,
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
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }

    let bounds = ClipRect { x0, y0, x1, y1 };
    (!bounds.is_empty()).then_some(bounds)
}

#[cfg_attr(not(test), allow(dead_code))]
fn node_affine_transform_matrix(layout: LayoutBox, transform: &Transform2D) -> AffineTransform {
    let matrix = node_local_transform_matrix(layout, transform);
    debug_assert!(matrix.is_2d());
    affine_from_2d_matrix(matrix)
}

pub(crate) fn node_local_transform_matrix(
    layout: LayoutBox,
    transform: &Transform2D,
) -> TransformMatrix3d {
    let origin_x = layout.x + transform.origin.x.resolve(layout.width);
    let origin_y = layout.y + transform.origin.y.resolve(layout.height);
    let mut matrix = TransformMatrix3d::IDENTITY;
    for operation in &transform.operations {
        matrix = transform_operation_matrix(layout, *operation).multiply(matrix);
    }

    TransformMatrix3d::translate(origin_x, origin_y, 0.0)
        .multiply(matrix)
        .multiply(TransformMatrix3d::translate(-origin_x, -origin_y, 0.0))
}

pub(crate) fn project_world_transform_matrix(
    layout: LayoutBox,
    world_matrix: TransformMatrix3d,
    perspective: Option<PerspectiveContext>,
) -> Option<AffineTransform> {
    if world_matrix.is_2d() {
        return Some(affine_from_2d_matrix(world_matrix));
    }

    let source = rect_corners(LayoutBox::new(0.0, 0.0, layout.width, layout.height));
    let mut destination = [(0.0, 0.0); 4];
    for (index, &(x, y)) in source.iter().enumerate() {
        let source_x = layout.x + x;
        let source_y = layout.y + y;
        let (projected_x, projected_y, _) =
            project_world_point(world_matrix, perspective, source_x, source_y, 0.0)?;
        destination[index] = (projected_x, projected_y);
    }

    homography_from_points(source, destination)
        .map(|matrix| matrix.multiply(AffineTransform::translate(-layout.x, -layout.y)))
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn project_transform_point(
    layout: LayoutBox,
    transform: &Transform2D,
    perspective: Option<PerspectiveContext>,
    x: f32,
    y: f32,
) -> Option<(f32, f32, f32)> {
    project_world_point(
        node_local_transform_matrix(layout, transform),
        perspective,
        x,
        y,
        0.0,
    )
}

pub(crate) fn project_world_point(
    world_matrix: TransformMatrix3d,
    perspective: Option<PerspectiveContext>,
    x: f32,
    y: f32,
    z: f32,
) -> Option<(f32, f32, f32)> {
    let (mut point_x, mut point_y, mut point_z, point_w) =
        world_matrix.transform_point(x, y, z, 1.0);
    if point_w <= TRANSFORM_EPSILON {
        return None;
    }

    if (point_w - 1.0).abs() > TRANSFORM_EPSILON {
        point_x /= point_w;
        point_y /= point_w;
        point_z /= point_w;
    }

    if let Some(perspective) = perspective {
        let denominator = 1.0 - point_z / perspective.depth;
        if denominator <= TRANSFORM_EPSILON {
            return None;
        }

        let relative_x = point_x - perspective.origin_x;
        let relative_y = point_y - perspective.origin_y;
        point_x = perspective.origin_x + relative_x / denominator;
        point_y = perspective.origin_y + relative_y / denominator;
    }

    Some((point_x, point_y, point_z))
}

fn affine_from_2d_matrix(matrix: TransformMatrix3d) -> AffineTransform {
    AffineTransform {
        a: matrix.m11,
        b: matrix.m21,
        c: matrix.m12,
        d: matrix.m22,
        e: matrix.m14,
        f: matrix.m24,
        g: 0.0,
        h: 0.0,
        i: 1.0,
    }
}

fn transform_operation_matrix(
    layout: LayoutBox,
    operation: TransformOperation,
) -> TransformMatrix3d {
    match operation {
        TransformOperation::Translate { x, y } => {
            TransformMatrix3d::translate(x.resolve(layout.width), y.resolve(layout.height), 0.0)
        }
        TransformOperation::TranslateZ { z } => TransformMatrix3d::translate(0.0, 0.0, z),
        TransformOperation::Scale { x, y } => TransformMatrix3d::scale(x, y, 1.0),
        TransformOperation::Rotate { degrees } | TransformOperation::RotateZ { degrees } => {
            TransformMatrix3d::rotate(0.0, 0.0, 1.0, degrees)
        }
        TransformOperation::RotateX { degrees } => {
            TransformMatrix3d::rotate(1.0, 0.0, 0.0, degrees)
        }
        TransformOperation::RotateY { degrees } => {
            TransformMatrix3d::rotate(0.0, 1.0, 0.0, degrees)
        }
        TransformOperation::Matrix3d { matrix } => matrix,
    }
}

fn rect_corners(layout: LayoutBox) -> [(f32, f32); 4] {
    [
        (layout.x, layout.y),
        (layout.x + layout.width, layout.y),
        (layout.x, layout.y + layout.height),
        (layout.x + layout.width, layout.y + layout.height),
    ]
}

fn homography_from_points(
    source: [(f32, f32); 4],
    destination: [(f32, f32); 4],
) -> Option<AffineTransform> {
    let mut matrix = [[0.0; 8]; 8];
    let mut vector = [0.0; 8];

    for (index, ((x, y), (u, v))) in source.into_iter().zip(destination).enumerate() {
        let row = index * 2;
        matrix[row] = [x, 0.0, y, 0.0, 1.0, 0.0, -u * x, -u * y];
        vector[row] = u;
        matrix[row + 1] = [0.0, x, 0.0, y, 0.0, 1.0, -v * x, -v * y];
        vector[row + 1] = v;
    }

    let solved = solve_linear_system(&mut matrix, &mut vector)?;
    Some(AffineTransform {
        a: solved[0],
        b: solved[1],
        c: solved[2],
        d: solved[3],
        e: solved[4],
        f: solved[5],
        g: solved[6],
        h: solved[7],
        i: 1.0,
    })
}

fn solve_linear_system(matrix: &mut [[f32; 8]; 8], vector: &mut [f32; 8]) -> Option<[f32; 8]> {
    for pivot in 0..8 {
        let mut best_row = pivot;
        let mut best_value = matrix[pivot][pivot].abs();
        for row in (pivot + 1)..8 {
            let candidate = matrix[row][pivot].abs();
            if candidate > best_value {
                best_row = row;
                best_value = candidate;
            }
        }

        if best_value <= TRANSFORM_EPSILON {
            return None;
        }

        if best_row != pivot {
            matrix.swap(pivot, best_row);
            vector.swap(pivot, best_row);
        }

        let pivot_value = matrix[pivot][pivot];
        for column in pivot..8 {
            matrix[pivot][column] /= pivot_value;
        }
        vector[pivot] /= pivot_value;

        for row in 0..8 {
            if row == pivot {
                continue;
            }
            let factor = matrix[row][pivot];
            if factor.abs() <= TRANSFORM_EPSILON {
                continue;
            }
            for column in pivot..8 {
                matrix[row][column] -= factor * matrix[pivot][column];
            }
            vector[row] -= factor * vector[pivot];
        }
    }

    Some(*vector)
}

fn layout_contains(layout: LayoutBox, x: f32, y: f32) -> bool {
    x >= layout.x && y >= layout.y && x < layout.x + layout.width && y < layout.y + layout.height
}

#[cfg(test)]
mod tests {
    use cssimpler_core::{
        LayoutBox, LengthPercentageValue, Transform2D, TransformMatrix3d, TransformOperation,
        TransformOrigin,
    };

    use super::{
        PerspectiveContext, node_local_transform_matrix, node_transform_matrix,
        project_transform_point, project_world_point, project_world_transform_matrix,
    };

    #[test]
    fn project_transform_point_supports_scale3d_depth_scaling() {
        let layout = LayoutBox::new(0.0, 0.0, 100.0, 100.0);
        let transform = Transform2D {
            origin: TransformOrigin {
                x: LengthPercentageValue::ZERO,
                y: LengthPercentageValue::ZERO,
            },
            operations: vec![
                TransformOperation::TranslateZ { z: 10.0 },
                TransformOperation::Matrix3d {
                    matrix: TransformMatrix3d::scale(1.0, 1.0, 2.0),
                },
            ],
        };

        let (_, _, z) = project_transform_point(layout, &transform, None, 25.0, 40.0)
            .expect("scale3d transform should project");

        assert!((z - 20.0).abs() < 0.001);
    }

    #[test]
    fn project_transform_point_supports_perspective_transform_function() {
        let layout = LayoutBox::new(0.0, 0.0, 100.0, 100.0);
        let transform = Transform2D {
            origin: TransformOrigin {
                x: LengthPercentageValue::ZERO,
                y: LengthPercentageValue::ZERO,
            },
            operations: vec![
                TransformOperation::TranslateZ { z: 10.0 },
                TransformOperation::Matrix3d {
                    matrix: TransformMatrix3d::perspective(100.0)
                        .expect("perspective matrix should build"),
                },
            ],
        };

        let (x, y, z) = project_transform_point(layout, &transform, None, 50.0, 20.0)
            .expect("perspective transform should project");

        assert!((x - 55.555557).abs() < 0.001);
        assert!((y - 22.222223).abs() < 0.001);
        assert!((z - 11.111112).abs() < 0.001);
    }

    #[test]
    fn node_transform_matrix_supports_affine_matrix3d_translation() {
        let layout = LayoutBox::new(10.0, 15.0, 80.0, 60.0);
        let transform = Transform2D {
            origin: TransformOrigin {
                x: LengthPercentageValue::ZERO,
                y: LengthPercentageValue::ZERO,
            },
            operations: vec![TransformOperation::Matrix3d {
                matrix: TransformMatrix3d::translate(12.0, 24.0, 0.0),
            }],
        };

        let matrix = node_transform_matrix(layout, &transform, None)
            .expect("2d matrix3d translation should convert to affine");
        let (x, y) = matrix.transform_point(layout.x, layout.y);

        assert!((x - 22.0).abs() < 0.001);
        assert!((y - 39.0).abs() < 0.001);
    }

    #[test]
    fn project_transform_point_combines_matrix3d_with_parent_perspective() {
        let layout = LayoutBox::new(0.0, 0.0, 100.0, 100.0);
        let transform = Transform2D {
            origin: TransformOrigin::default(),
            operations: vec![
                TransformOperation::Matrix3d {
                    matrix: TransformMatrix3d::rotate(0.0, 1.0, 0.0, 20.0),
                },
                TransformOperation::Matrix3d {
                    matrix: TransformMatrix3d::scale(1.02, 1.02, 1.02),
                },
            ],
        };

        let projected = project_transform_point(
            layout,
            &transform,
            Some(PerspectiveContext {
                depth: 800.0,
                origin_x: 50.0,
                origin_y: 50.0,
            }),
            100.0,
            50.0,
        )
        .expect("combined scale3d and perspective should project");

        assert!(projected.0 < 100.0);
        assert!(projected.0 > 90.0);
        assert!(projected.2 < 0.0);
    }

    #[test]
    fn nested_child_depth_translation_uses_the_parents_3d_rotation() {
        let parent_layout = LayoutBox::new(0.0, 0.0, 200.0, 200.0);
        let child_layout = LayoutBox::new(90.0, 90.0, 20.0, 20.0);
        let parent_transform = Transform2D {
            origin: TransformOrigin::default(),
            operations: vec![TransformOperation::RotateY { degrees: -30.0 }],
        };
        let child_transform = Transform2D {
            origin: TransformOrigin::default(),
            operations: vec![TransformOperation::TranslateZ { z: 40.0 }],
        };

        let world_matrix = node_local_transform_matrix(parent_layout, &parent_transform)
            .multiply(node_local_transform_matrix(child_layout, &child_transform));
        let projected = project_world_point(
            world_matrix,
            Some(PerspectiveContext {
                depth: 1000.0,
                origin_x: 100.0,
                origin_y: 100.0,
            }),
            100.0,
            100.0,
            0.0,
        )
        .expect("nested 3d transform should project");

        assert!(projected.0 < 90.0);
        assert!(projected.2 > 30.0);
    }

    #[test]
    fn centered_rotate_x_and_y_keep_the_pivot_fixed_under_perspective() {
        let layout = LayoutBox::new(50.0, 40.0, 120.0, 160.0);
        let center_x = layout.x + layout.width * 0.5;
        let center_y = layout.y + layout.height * 0.5;
        let transform = Transform2D {
            origin: TransformOrigin::default(),
            operations: vec![
                TransformOperation::RotateX { degrees: 32.0 },
                TransformOperation::RotateY { degrees: -24.0 },
            ],
        };

        let projected = project_transform_point(
            layout,
            &transform,
            Some(PerspectiveContext {
                depth: 720.0,
                origin_x: center_x,
                origin_y: center_y,
            }),
            center_x,
            center_y,
        )
        .expect("centered rotateX/rotateY transform should project");

        assert!((projected.0 - center_x).abs() < 0.001);
        assert!((projected.1 - center_y).abs() < 0.001);
        assert!(projected.2.abs() < 0.001);
    }

    #[test]
    fn offset_rotate_y_keeps_projected_corners_broad() {
        let layout = LayoutBox::new(450.0, 153.0, 220.0, 280.0);
        let transform = Transform2D {
            origin: TransformOrigin::default(),
            operations: vec![TransformOperation::RotateY { degrees: 15.0 }],
        };
        let perspective = Some(PerspectiveContext {
            depth: 950.0,
            origin_x: 560.0,
            origin_y: 293.0,
        });

        let left = project_transform_point(layout, &transform, perspective, layout.x, layout.y)
            .expect("left corner should project");
        let right = project_transform_point(
            layout,
            &transform,
            perspective,
            layout.x + layout.width,
            layout.y,
        )
        .expect("right corner should project");

        assert!(
            right.0 - left.0 > 180.0,
            "projected corners should remain broad, got left={left:?}, right={right:?}"
        );
    }

    #[test]
    fn projected_world_transform_matrix_maps_offset_rotate_y_corners_correctly() {
        let layout = LayoutBox::new(450.0, 153.0, 220.0, 280.0);
        let transform = Transform2D {
            origin: TransformOrigin::default(),
            operations: vec![TransformOperation::RotateY { degrees: 15.0 }],
        };
        let perspective = Some(PerspectiveContext {
            depth: 950.0,
            origin_x: 560.0,
            origin_y: 293.0,
        });
        let world_matrix = node_local_transform_matrix(layout, &transform);
        let projected = project_world_transform_matrix(layout, world_matrix, perspective)
            .expect("projected matrix should exist");

        let expected_top_left =
            project_world_point(world_matrix, perspective, layout.x, layout.y, 0.0)
                .expect("top-left should project");
        let expected_top_right = project_world_point(
            world_matrix,
            perspective,
            layout.x + layout.width,
            layout.y,
            0.0,
        )
        .expect("top-right should project");

        let mapped_top_left = projected.transform_point(layout.x, layout.y);
        let mapped_top_right = projected.transform_point(layout.x + layout.width, layout.y);

        assert!(
            (mapped_top_left.0 - expected_top_left.0).abs() < 0.01
                && (mapped_top_left.1 - expected_top_left.1).abs() < 0.01,
            "top-left mismatch: mapped={mapped_top_left:?}, expected={expected_top_left:?}"
        );
        assert!(
            (mapped_top_right.0 - expected_top_right.0).abs() < 0.01
                && (mapped_top_right.1 - expected_top_right.1).abs() < 0.01,
            "top-right mismatch: mapped={mapped_top_right:?}, expected={expected_top_right:?}"
        );
    }

    #[test]
    fn projected_world_transform_matrix_inverse_recovers_offset_rotate_y_points() {
        let layout = LayoutBox::new(450.0, 153.0, 220.0, 280.0);
        let transform = Transform2D {
            origin: TransformOrigin::default(),
            operations: vec![TransformOperation::RotateY { degrees: 15.0 }],
        };
        let perspective = Some(PerspectiveContext {
            depth: 950.0,
            origin_x: 560.0,
            origin_y: 293.0,
        });
        let world_matrix = node_local_transform_matrix(layout, &transform);
        let projected = project_world_transform_matrix(layout, world_matrix, perspective)
            .expect("projected matrix should exist");
        let inverse = projected.invert().expect("projected matrix should invert");
        let source_center = (
            layout.x + layout.width * 0.5,
            layout.y + layout.height * 0.5,
        );
        let projected_center = projected.transform_point(source_center.0, source_center.1);
        let recovered_center = inverse.transform_point(projected_center.0, projected_center.1);

        assert!(
            (recovered_center.0 - source_center.0).abs() < 0.01
                && (recovered_center.1 - source_center.1).abs() < 0.01,
            "inverse should recover the source center, got recovered={recovered_center:?}, source={source_center:?}, projected={projected_center:?}"
        );
    }
}
