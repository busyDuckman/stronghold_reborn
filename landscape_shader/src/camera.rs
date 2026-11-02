// --------------------------------------------------------------------------------------------------------------------
// A Doom-style camera: a position on the map, a heading, a height and a field of view. No
// pitch. It walks with the arrow keys and either rides the ground at an eye height or holds
// an altitude of its own.
//
// World space for every renderer: x and y are cells (x right, y down the map as the top-down
// view has it), z is up and is height units times the height scale in the view options.

use crate::game_map::GameMap;

/// The keys held this frame, as the camera understands them.
#[derive(Copy, Clone, Default, PartialEq, Eq, Debug)]
pub struct CameraInput {
    pub forward: bool,
    pub back: bool,
    pub turn_left: bool,
    pub turn_right: bool,
    pub strafe_left: bool,
    pub strafe_right: bool,
    pub rise: bool,
    pub sink: bool,
    pub fov_wider: bool,
    pub fov_narrower: bool,
    pub fast: bool,
}

impl CameraInput {
    pub fn any(&self) -> bool {
        *self != CameraInput::default()
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Camera {
    /// Position in cells.
    pub x: f32,
    pub y: f32,
    /// Heading in radians; 0 looks along +x, and it turns towards +y (down the map) as it grows.
    pub yaw: f32,
    /// Eye height above the ground, in height units, when following the ground.
    pub eye_height: f32,
    /// Absolute height in height units when not following the ground.
    pub altitude: f32,
    pub follow_ground: bool,
    /// Horizontal field of view in degrees.
    pub fov_degrees: f32,
    /// Cells per second, and radians per second.
    pub move_speed: f32,
    pub turn_speed: f32,
}

/// The FOV the camera is held between.
pub const FOV_MIN: f32 = 30.0;
pub const FOV_MAX: f32 = 150.0;

impl Default for Camera {
    fn default() -> Camera {
        Camera {
            x: 74.0,
            y: 30.0,
            yaw: 0.0,
            eye_height: 6.0,
            altitude: 60.0,
            follow_ground: true,
            fov_degrees: 90.0,
            move_speed: 12.0,
            turn_speed: 1.6,
        }
    }
}

impl Camera {
    /// Stand in the middle of a map, looking along +x.
    pub fn at_map_centre(map: &GameMap) -> Camera {
        Camera { x: map.cols() as f32 * 0.5, y: map.rows() as f32 * 0.5, ..Camera::default() }
    }

    /// The unit vector the camera looks along, in the map plane.
    pub fn forward(&self) -> [f32; 2] {
        [self.yaw.cos(), self.yaw.sin()]
    }

    /// The unit vector to the camera's right, in the map plane.
    pub fn right(&self) -> [f32; 2] {
        // Right is what a reader of the top-down map calls right: facing east (+x) it is
        // south (+y, down the map). The frame x right, y down, z up is left-handed when seen
        // from above, so this is forward turned a quarter turn towards +y, not a cross
        // product with up; that cross product mirrors the world and swaps the turn keys.
        [-self.yaw.sin(), self.yaw.cos()]
    }

    /// The eye's height in height units: ground plus eye height, or the altitude.
    pub fn eye_height_units(&self, map: &GameMap) -> f32 {
        if self.follow_ground {
            map.height_bilinear(self.x, self.y) + self.eye_height
        } else {
            self.altitude
        }
    }

    /// The eye in world space.
    pub fn eye(&self, map: &GameMap, height_scale: f32) -> [f32; 3] {
        [self.x, self.y, self.eye_height_units(map) * height_scale]
    }

    /// Move for one frame of held keys. Position is kept on the map.
    pub fn apply(&mut self, input: &CameraInput, dt: f32, map: &GameMap) {
        let speed = if input.fast { self.move_speed * 3.0 } else { self.move_speed };
        let step = speed * dt;
        let [fx, fy] = self.forward();
        let [rx, ry] = self.right();
        if input.forward {
            self.x += fx * step;
            self.y += fy * step;
        }
        if input.back {
            self.x -= fx * step;
            self.y -= fy * step;
        }
        if input.strafe_right {
            self.x += rx * step;
            self.y += ry * step;
        }
        if input.strafe_left {
            self.x -= rx * step;
            self.y -= ry * step;
        }
        if input.turn_left {
            self.yaw -= self.turn_speed * dt;
        }
        if input.turn_right {
            self.yaw += self.turn_speed * dt;
        }
        let climb = 20.0 * dt;
        if input.rise {
            self.eye_height += climb;
            self.altitude += climb;
        }
        if input.sink {
            self.eye_height = (self.eye_height - climb).max(0.5);
            self.altitude = (self.altitude - climb).max(0.5);
        }
        if input.fov_wider {
            self.fov_degrees = (self.fov_degrees + 40.0 * dt).min(FOV_MAX);
        }
        if input.fov_narrower {
            self.fov_degrees = (self.fov_degrees - 40.0 * dt).max(FOV_MIN);
        }
        self.yaw = self.yaw.rem_euclid(std::f32::consts::TAU);
        self.x = self.x.clamp(0.0, map.cols() as f32);
        self.y = self.y.clamp(0.0, map.rows() as f32);
    }

    /// The view-projection matrix, row major, for a target of the given aspect. Depth lands
    /// in 0..=1 as wgpu wants it; the FOV is horizontal.
    pub fn view_projection(&self, eye: [f32; 3], aspect: f32, near: f32, far: f32) -> Mat4 {
        let [fx, fy] = self.forward();
        let [rx, ry] = self.right();
        // Rows: right, up, back (the camera looks down its -z), then the eye moved to the origin.
        // Because the world frame is left-handed seen from above this view is a reflection,
        // which is harmless: nothing here culls by winding order.
        let view = Mat4([
            [rx, ry, 0.0, -(rx * eye[0] + ry * eye[1])],
            [0.0, 0.0, 1.0, -eye[2]],
            [-fx, -fy, 0.0, fx * eye[0] + fy * eye[1]],
            [0.0, 0.0, 0.0, 1.0],
        ]);
        let half_x = (self.fov_degrees.clamp(FOV_MIN, FOV_MAX).to_radians() * 0.5).tan();
        let half_y = half_x / aspect.max(1e-3);
        let projection = Mat4([
            [1.0 / half_x, 0.0, 0.0, 0.0],
            [0.0, 1.0 / half_y, 0.0, 0.0],
            [0.0, 0.0, far / (near - far), near * far / (near - far)],
            [0.0, 0.0, -1.0, 0.0],
        ]);
        projection.mul(&view)
    }
}

// --------------------------------------------------------------------------------------------------------------------
// A small 4x4 matrix, row major

#[derive(Copy, Clone, PartialEq, Debug)]
pub struct Mat4(pub [[f32; 4]; 4]);

impl Mat4 {
    pub fn mul(&self, other: &Mat4) -> Mat4 {
        let mut out = [[0.0f32; 4]; 4];
        for (r, row) in out.iter_mut().enumerate() {
            for (c, cell) in row.iter_mut().enumerate() {
                *cell = (0..4).map(|k| self.0[r][k] * other.0[k][c]).sum();
            }
        }
        Mat4(out)
    }

    /// Transform a point, returning the clip-space result (x, y, z, w).
    pub fn transform(&self, p: [f32; 3]) -> [f32; 4] {
        let v = [p[0], p[1], p[2], 1.0];
        let mut out = [0.0f32; 4];
        for (r, value) in out.iter_mut().enumerate() {
            *value = (0..4).map(|k| self.0[r][k] * v[k]).sum();
        }
        out
    }

    /// Column major floats, the layout a WGSL mat4x4 reads.
    pub fn to_columns(&self) -> [f32; 16] {
        let mut out = [0.0f32; 16];
        for c in 0..4 {
            for r in 0..4 {
                out[c * 4 + r] = self.0[r][c];
            }
        }
        out
    }
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;

    fn flat_map() -> GameMap {
        let mut map = GameMap::new_flat(20, 10).expect("map");
        for y in 0..10 {
            for x in 0..20 {
                map.set_height(x, y, 10).expect("in range");
            }
        }
        map
    }

    #[test]
    fn walking_forward_moves_along_the_heading() {
        let map = flat_map();
        let mut camera = Camera { x: 5.0, y: 5.0, yaw: 0.0, move_speed: 2.0, ..Camera::default() };
        camera.apply(&CameraInput { forward: true, ..CameraInput::default() }, 0.5, &map);
        assert!((camera.x - 6.0).abs() < 1e-5 && (camera.y - 5.0).abs() < 1e-5);
        camera.yaw = std::f32::consts::FRAC_PI_2;
        camera.apply(&CameraInput { forward: true, ..CameraInput::default() }, 0.5, &map);
        assert!((camera.x - 6.0).abs() < 1e-5 && (camera.y - 6.0).abs() < 1e-5);
        camera.apply(&CameraInput { forward: true, fast: true, ..CameraInput::default() }, 100.0, &map);
        assert_eq!(camera.y, 10.0, "held on the map");
    }

    #[test]
    fn the_turn_keys_match_the_map() {
        let map = flat_map();
        // Facing east, the right key swings the heading towards south (+y, down the map),
        // and strafing right walks that way too.
        let mut camera = Camera { x: 5.0, y: 5.0, yaw: 0.0, turn_speed: 1.0, move_speed: 2.0, ..Camera::default() };
        camera.apply(&CameraInput { turn_right: true, ..CameraInput::default() }, 0.5, &map);
        assert!(camera.forward()[1] > 0.0, "heading {:?}", camera.forward());
        camera.yaw = 0.0;
        camera.apply(&CameraInput { strafe_right: true, ..CameraInput::default() }, 0.5, &map);
        assert!((camera.y - 6.0).abs() < 1e-5 && (camera.x - 5.0).abs() < 1e-5);
    }

    #[test]
    fn eye_follows_the_ground_or_holds_altitude() {
        let map = flat_map();
        let mut camera = Camera { x: 5.0, y: 5.0, eye_height: 4.0, altitude: 50.0, follow_ground: true, ..Camera::default() };
        assert_eq!(camera.eye_height_units(&map), 14.0);
        assert_eq!(camera.eye(&map, 0.1)[2], 1.4);
        camera.follow_ground = false;
        assert_eq!(camera.eye_height_units(&map), 50.0);
    }

    #[test]
    fn a_point_ahead_projects_to_the_centre_with_depth_in_range() {
        let camera = Camera { x: 0.0, y: 0.0, yaw: 0.0, fov_degrees: 90.0, ..Camera::default() };
        let vp = camera.view_projection([0.0, 0.0, 1.0], 2.0, 0.1, 100.0);
        let ahead = vp.transform([10.0, 0.0, 1.0]);
        let (x, y, z) = (ahead[0] / ahead[3], ahead[1] / ahead[3], ahead[2] / ahead[3]);
        assert!(x.abs() < 1e-5 && y.abs() < 1e-5, "{x} {y}");
        assert!((0.0..=1.0).contains(&z), "depth {z}");
        // A point to the right (south, down the map, when facing east) lands on the right of
        // the screen; one higher lands higher.
        let right = vp.transform([10.0, 10.0, 1.0]);
        assert!(right[0] / right[3] > 0.0);
        let up = vp.transform([10.0, 0.0, 5.0]);
        assert!(up[1] / up[3] > 0.0);
        // Behind the camera has negative w.
        assert!(vp.transform([-10.0, 0.0, 1.0])[3] < 0.0);
    }

    #[test]
    fn columns_transpose_the_rows() {
        let m = Mat4([[1.0, 2.0, 3.0, 4.0], [5.0, 6.0, 7.0, 8.0], [9.0, 10.0, 11.0, 12.0], [13.0, 14.0, 15.0, 16.0]]);
        assert_eq!(m.to_columns()[1], 5.0);
        assert_eq!(m.to_columns()[4], 2.0);
    }
}
