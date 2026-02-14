use std::time::Instant;

const CROSSFADE_DURATION_MS: f32 = 250.0;

pub struct ViewerState {
    pub current_index: Option<usize>,
    pub transition: Option<Transition>,
    pub show_info: bool,
    pub zoom_level: f32,
    pub zoom_target: f32,
    pub zoom_offset: (f32, f32),
    /// Cursor position in window coordinates when zoom was initiated.
    /// Used to keep the point under the cursor fixed during zoom animation.
    pub zoom_anchor: Option<(f32, f32)>,
}

impl Default for ViewerState {
    fn default() -> Self {
        Self {
            current_index: None,
            transition: None,
            show_info: false,
            zoom_level: 1.0,
            zoom_target: 1.0,
            zoom_offset: (0.0, 0.0),
            zoom_anchor: None,
        }
    }
}

pub struct Transition {
    pub from_index: usize,
    pub start: Instant,
}

impl ViewerState {
    pub fn open_index(&mut self, index: usize) {
        self.transition = None;
        self.current_index = Some(index);
    }

    pub fn close(&mut self) {
        self.current_index = None;
        self.transition = None;
        self.reset_zoom();
    }

    pub fn toggle_info(&mut self) {
        self.show_info = !self.show_info;
    }

    pub fn is_zoomed(&self) -> bool {
        self.zoom_level > 1.0
    }

    pub fn is_zoom_animating(&self) -> bool {
        (self.zoom_level - self.zoom_target).abs() > 0.005
    }

    pub fn toggle_zoom(&mut self) {
        if self.zoom_target > 1.0 {
            self.zoom_target = 1.0;
        } else {
            self.zoom_target = 2.0;
        }
        self.zoom_offset = (0.0, 0.0);
        self.zoom_anchor = None;
    }

    pub fn reset_zoom(&mut self) {
        self.zoom_level = 1.0;
        self.zoom_target = 1.0;
        self.zoom_offset = (0.0, 0.0);
        self.zoom_anchor = None;
    }

    /// Set zoom target from a scroll delta. The actual zoom_level is animated
    /// toward this target on each tick.
    pub fn adjust_zoom(&mut self, delta: f32) {
        let factor = 2.0_f32.powf(delta * 0.15);
        self.zoom_target = (self.zoom_target * factor).clamp(1.0, 8.0);
        if self.zoom_target < 1.02 {
            self.zoom_target = 1.0;
        }
    }

    /// Animate zoom_level toward zoom_target. Returns true if zoom just
    /// crossed from <=1.0 to >1.0 (scrollable needs centering).
    pub fn tick_zoom(&mut self) -> bool {
        if !self.is_zoom_animating() {
            self.zoom_level = self.zoom_target;
            if self.zoom_level < 1.02 && self.zoom_target <= 1.0 {
                self.zoom_level = 1.0;
                self.zoom_target = 1.0;
                self.zoom_offset = (0.0, 0.0);
            }
            return false;
        }
        let was_zoomed = self.is_zoomed();
        // Exponential easing — cover 25% of remaining distance per frame (~60fps)
        self.zoom_level += (self.zoom_target - self.zoom_level) * 0.25;
        // Snap when very close
        if (self.zoom_level - self.zoom_target).abs() < 0.005 {
            self.zoom_level = self.zoom_target;
        }
        if self.zoom_level < 1.02 && self.zoom_target <= 1.0 {
            self.zoom_level = 1.0;
            self.zoom_target = 1.0;
            self.zoom_offset = (0.0, 0.0);
        }
        !was_zoomed && self.is_zoomed()
    }

    pub fn navigate_to(&mut self, new_index: usize) {
        if let Some(old_index) = self.current_index {
            if old_index != new_index {
                self.current_index = Some(new_index);
                self.reset_zoom();
            }
        }
    }

    pub fn next(&mut self, total: usize) {
        if let Some(i) = self.current_index {
            if i + 1 < total {
                self.navigate_to(i + 1);
            }
        }
    }

    pub fn prev(&mut self) {
        if let Some(i) = self.current_index {
            if i > 0 {
                self.navigate_to(i - 1);
            }
        }
    }

    /// Returns the crossfade progress (0.0 = just started, 1.0 = done).
    /// Returns None if no transition is active.
    pub fn transition_progress(&self) -> Option<f32> {
        let t = self.transition.as_ref()?;
        let elapsed = t.start.elapsed().as_millis() as f32;
        let progress = (elapsed / CROSSFADE_DURATION_MS).min(1.0);
        Some(progress)
    }

    pub fn is_transitioning(&self) -> bool {
        matches!(self.transition_progress(), Some(p) if p < 1.0)
    }

    pub fn tick(&mut self) {
        if let Some(progress) = self.transition_progress() {
            if progress >= 1.0 {
                self.transition = None;
            }
        }
    }
}
