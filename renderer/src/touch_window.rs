//! Window gestures use physical screen coordinates. A finger remains the owner
//! until release/cancel, even when it moves out of the original resize border.
use winit::{
    dpi::{PhysicalPosition, PhysicalSize},
    window::ResizeDirection,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Action {
    Move,
    Resize(ResizeDirection),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Geometry {
    pub position: PhysicalPosition<i32>,
    pub size: PhysicalSize<u32>,
}

pub(super) struct Gesture {
    pub id: u64,
    pub action: Action,
    start: PhysicalPosition<f64>,
    original: Geometry,
}

impl Gesture {
    pub fn new(id: u64, action: Action, start: PhysicalPosition<f64>, original: Geometry) -> Self {
        Self {
            id,
            action,
            start,
            original,
        }
    }

    pub fn geometry_at(&self, position: PhysicalPosition<f64>) -> Geometry {
        let dx = (position.x - self.start.x).round() as i64;
        let dy = (position.y - self.start.y).round() as i64;
        let Geometry { position, size } = self.original;
        let (mut x, mut y) = (i64::from(position.x), i64::from(position.y));
        let (mut width, mut height) = (i64::from(size.width), i64::from(size.height));
        match self.action {
            Action::Move => {
                x += dx;
                y += dy;
            }
            Action::Resize(direction) => {
                use ResizeDirection::*;
                if matches!(direction, West | NorthWest | SouthWest) {
                    // Keep the opposite edge fixed when a finger crosses it.
                    let delta = dx.min(width - 1);
                    x += delta;
                    width -= delta;
                } else if matches!(direction, East | NorthEast | SouthEast) {
                    width = (width + dx).max(1);
                }
                if matches!(direction, North | NorthWest | NorthEast) {
                    let delta = dy.min(height - 1);
                    y += delta;
                    height -= delta;
                } else if matches!(direction, South | SouthWest | SouthEast) {
                    height = (height + dy).max(1);
                }
            }
        }
        Geometry {
            position: PhysicalPosition::new(
                x.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
                y.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
            ),
            size: PhysicalSize::new(
                width.clamp(1, i64::from(i32::MAX)) as u32,
                height.clamp(1, i64::from(i32::MAX)) as u32,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gesture(action: Action) -> Gesture {
        Gesture::new(
            7,
            action,
            PhysicalPosition::new(120.0, 220.0),
            Geometry {
                position: PhysicalPosition::new(100, 200),
                size: PhysicalSize::new(800, 600),
            },
        )
    }

    #[test]
    fn moving_uses_the_finger_screen_position_without_scaling_twice() {
        let drag = gesture(Action::Move);
        assert_eq!(drag.id, 7);
        let geometry = drag.geometry_at(PhysicalPosition::new(180.0, 250.0));
        assert_eq!(geometry.position, PhysicalPosition::new(160, 230));
        assert_eq!(geometry.size, PhysicalSize::new(800, 600));
        // After the window moves, the same finger can be at the same client
        // coordinates while continuing to move in physical screen coordinates.
        let next = drag.geometry_at(PhysicalPosition::new(200.0, 260.0));
        assert_eq!(next.position, PhysicalPosition::new(180, 240));
    }

    #[test]
    fn all_edges_and_corners_keep_the_opposite_edges_fixed() {
        use ResizeDirection::*;
        for (direction, expected) in [
            (North, (100, 230, 800, 570)),
            (NorthEast, (100, 230, 860, 570)),
            (East, (100, 200, 860, 600)),
            (SouthEast, (100, 200, 860, 630)),
            (South, (100, 200, 800, 630)),
            (SouthWest, (160, 200, 740, 630)),
            (West, (160, 200, 740, 600)),
            (NorthWest, (160, 230, 740, 570)),
        ] {
            let bounds =
                gesture(Action::Resize(direction)).geometry_at(PhysicalPosition::new(180.0, 250.0));
            assert_eq!(
                (
                    bounds.position.x,
                    bounds.position.y,
                    bounds.size.width,
                    bounds.size.height
                ),
                expected,
                "{direction:?}"
            );
        }
    }

    #[test]
    fn crossing_opposite_edges_cannot_invert_the_window() {
        let bounds = gesture(Action::Resize(ResizeDirection::NorthWest))
            .geometry_at(PhysicalPosition::new(2000.0, 2000.0));
        assert_eq!(bounds.position, PhysicalPosition::new(899, 799));
        assert_eq!(bounds.size, PhysicalSize::new(1, 1));
        let bounds = gesture(Action::Resize(ResizeDirection::SouthEast))
            .geometry_at(PhysicalPosition::new(-2000.0, -2000.0));
        assert_eq!(bounds.position, PhysicalPosition::new(100, 200));
        assert_eq!(bounds.size, PhysicalSize::new(1, 1));
    }

    #[test]
    fn moving_supports_monitors_with_negative_screen_coordinates() {
        let drag = Gesture::new(
            1,
            Action::Move,
            PhysicalPosition::new(-100.0, 10.0),
            Geometry {
                position: PhysicalPosition::new(-500, -200),
                size: PhysicalSize::new(400, 300),
            },
        );
        let bounds = drag.geometry_at(PhysicalPosition::new(-300.0, -50.0));
        assert_eq!(bounds.position, PhysicalPosition::new(-700, -260));
    }
}
