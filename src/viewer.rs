use std::time::Instant;

const CROSSFADE_DURATION_MS: f32 = 250.0;

#[derive(Default)]
pub struct ViewerState {
    pub current_index: Option<usize>,
    pub transition: Option<Transition>,
    pub show_info: bool,
    pub zoomed: bool,
    pub zoom_offset: (f32, f32),
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

    pub fn toggle_zoom(&mut self) {
        self.zoomed = !self.zoomed;
        self.zoom_offset = (0.0, 0.0);
    }

    pub fn reset_zoom(&mut self) {
        self.zoomed = false;
        self.zoom_offset = (0.0, 0.0);
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
