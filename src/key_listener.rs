//! A transparent wrapper widget that intercepts keyboard events synchronously.
//! Unlike subscription-based keyboard handling, messages are produced in the
//! same frame as the event — no async executor delay.
//!
//! Also supports scroll interception for zoom, mouse drag for panning, and
//! click/right-click callbacks.

use std::collections::HashMap;

use iced::advanced::layout;
use iced::advanced::overlay;
use iced::advanced::renderer;
use iced::advanced::widget::tree::Tag;
use iced::advanced::widget::{Operation, Tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget};
use iced::{keyboard, mouse, touch, Element, Event, Length, Point, Rectangle, Size, Vector};

const DRAG_THRESHOLD: f32 = 8.0;

#[derive(Debug, Default)]
struct State {
    /// Left button is currently held down.
    pressed: bool,
    /// Where the left button was pressed (for click vs drag detection).
    press_pos: Option<Point>,
    /// True once the cursor has moved beyond DRAG_THRESHOLD from press_pos.
    dragging: bool,
    /// Last cursor position (for computing drag deltas).
    last_pos: Option<Point>,
    /// Active touch finger positions (for pinch-to-zoom and touch drag).
    touches: HashMap<touch::Finger, Point>,
    /// Previous distance between two fingers during a pinch gesture.
    pinch_last_distance: Option<f32>,
    /// True after a pinch ends, to prevent the remaining finger from triggering drag.
    was_pinching: bool,
}

pub struct KeyListener<'a, Message, Theme = iced::Theme, Renderer = iced::Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
    on_key_press: Box<dyn Fn(keyboard::Key, bool) -> Option<Message> + 'a>,
    /// Called on scroll events with (delta, cursor_x, cursor_y).
    on_scroll: Option<Box<dyn Fn(f32, f32, f32) -> Option<Message> + 'a>>,
    /// Called on mouse drag with (dx, dy). Return Some to consume the event.
    on_drag: Option<Box<dyn Fn(f32, f32) -> Option<Message> + 'a>>,
    /// Called on left click (press+release without drag) with (cursor_x, cursor_y).
    on_click: Option<Box<dyn Fn(f32, f32) -> Option<Message> + 'a>>,
    /// Called on right click with (cursor_x, cursor_y).
    on_right_click: Option<Box<dyn Fn(f32, f32) -> Option<Message> + 'a>>,
    /// Called on pinch gesture with (scale, center_x, center_y).
    on_pinch: Option<Box<dyn Fn(f32, f32, f32) -> Option<Message> + 'a>>,
}

impl<'a, Message, Theme, Renderer> KeyListener<'a, Message, Theme, Renderer> {
    pub fn new(
        content: impl Into<Element<'a, Message, Theme, Renderer>>,
        on_key_press: impl Fn(keyboard::Key, bool) -> Option<Message> + 'a,
    ) -> Self {
        Self {
            content: content.into(),
            on_key_press: Box::new(on_key_press),
            on_scroll: None,
            on_drag: None,
            on_click: None,
            on_right_click: None,
            on_pinch: None,
        }
    }

    pub fn on_scroll(
        mut self,
        f: impl Fn(f32, f32, f32) -> Option<Message> + 'a,
    ) -> Self {
        self.on_scroll = Some(Box::new(f));
        self
    }

    pub fn on_drag(
        mut self,
        f: impl Fn(f32, f32) -> Option<Message> + 'a,
    ) -> Self {
        self.on_drag = Some(Box::new(f));
        self
    }

    pub fn on_click(
        mut self,
        f: impl Fn(f32, f32) -> Option<Message> + 'a,
    ) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }

    pub fn on_right_click(
        mut self,
        f: impl Fn(f32, f32) -> Option<Message> + 'a,
    ) -> Self {
        self.on_right_click = Some(Box::new(f));
        self
    }

    pub fn on_pinch(
        mut self,
        f: impl Fn(f32, f32, f32) -> Option<Message> + 'a,
    ) -> Self {
        self.on_pinch = Some(Box::new(f));
        self
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for KeyListener<'_, Message, Theme, Renderer>
where
    Renderer: iced::advanced::Renderer,
{
    fn tag(&self) -> Tag {
        Tag::of::<State>()
    }

    fn state(&self) -> iced::advanced::widget::tree::State {
        iced::advanced::widget::tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: iced::mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();

        // --- Pre-children: track press state and handle drag ---
        // Press tracking and drag must happen before children so we can
        // intercept the scrollable's own drag/scroll behavior.
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(pos) = cursor.position() {
                    state.pressed = true;
                    state.press_pos = Some(pos);
                    state.dragging = false;
                    state.last_pos = Some(pos);
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if state.pressed {
                    if let (Some(press), Some(pos)) = (state.press_pos, cursor.position()) {
                        if !state.dragging {
                            let dist = ((pos.x - press.x).powi(2) + (pos.y - press.y).powi(2)).sqrt();
                            if dist > DRAG_THRESHOLD {
                                state.dragging = true;
                            }
                        }
                        if state.dragging {
                            if let Some(last) = state.last_pos {
                                let dx = pos.x - last.x;
                                let dy = pos.y - last.y;
                                if dx.abs() > 0.5 || dy.abs() > 0.5 {
                                    if let Some(ref on_drag) = self.on_drag {
                                        state.last_pos = Some(pos);
                                        if let Some(message) = on_drag(dx, dy) {
                                            shell.publish(message);
                                            return;
                                        }
                                    }
                                }
                            }
                        }
                        state.last_pos = Some(pos);
                    }
                }
            }
            _ => {}
        }

        // --- Scroll interception (before children) ---
        if let Event::Mouse(mouse::Event::WheelScrolled { delta }) = event {
            if let Some(ref on_scroll) = self.on_scroll {
                let y = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => *y,
                    mouse::ScrollDelta::Pixels { y, .. } => *y / 40.0,
                };
                if y.abs() > 0.001 {
                    let (cx, cy) = cursor
                        .position()
                        .map(|p| (p.x, p.y))
                        .unwrap_or((0.0, 0.0));
                    if let Some(message) = on_scroll(y, cx, cy) {
                        shell.publish(message);
                        return;
                    }
                }
            }
        }

        // --- Touch interception (before children): single-finger drag + pinch ---
        if let Event::Touch(touch_event) = event {
            match touch_event {
                touch::Event::FingerPressed { id, position } => {
                    state.touches.insert(*id, *position);
                    if state.touches.len() == 2 {
                        let pts: Vec<Point> = state.touches.values().copied().collect();
                        let dist = distance(pts[0], pts[1]);
                        state.pinch_last_distance = Some(dist);
                    }
                }
                touch::Event::FingerMoved { id, position } => {
                    let old_pos = state.touches.get(id).copied();
                    state.touches.insert(*id, *position);
                    if state.touches.len() == 1 && !state.was_pinching {
                        // Single-finger drag — fire on_drag with deltas
                        if let Some(old) = old_pos {
                            let dx = position.x - old.x;
                            let dy = position.y - old.y;
                            if dx.abs() > 0.5 || dy.abs() > 0.5 {
                                if let Some(ref on_drag) = self.on_drag {
                                    if let Some(msg) = on_drag(dx, dy) {
                                        shell.publish(msg);
                                        return;
                                    }
                                }
                            }
                        }
                    } else if state.touches.len() == 2 {
                        if let Some(ref on_pinch) = self.on_pinch {
                            let pts: Vec<Point> = state.touches.values().copied().collect();
                            let dist = distance(pts[0], pts[1]);
                            if let Some(prev_dist) = state.pinch_last_distance {
                                if prev_dist > 1.0 && dist > 1.0 {
                                    let scale = dist / prev_dist;
                                    let cx = (pts[0].x + pts[1].x) / 2.0;
                                    let cy = (pts[0].y + pts[1].y) / 2.0;
                                    state.pinch_last_distance = Some(dist);
                                    if let Some(msg) = on_pinch(scale, cx, cy) {
                                        shell.publish(msg);
                                        return;
                                    }
                                }
                            }
                            state.pinch_last_distance = Some(dist);
                        }
                    }
                }
                touch::Event::FingerLifted { id, .. }
                | touch::Event::FingerLost { id, .. } => {
                    state.touches.remove(id);
                    if state.touches.len() < 2 {
                        if state.pinch_last_distance.is_some() {
                            state.was_pinching = true;
                        }
                        state.pinch_last_distance = None;
                    }
                    if state.touches.is_empty() {
                        state.was_pinching = false;
                    }
                }
            }
        }

        // --- Pass event to children ---
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );

        if shell.is_event_captured() {
            // A child widget (button, etc.) handled this event — clean up
            // press state but don't fire click callbacks.
            if matches!(
                event,
                Event::Mouse(
                    mouse::Event::ButtonPressed(..) | mouse::Event::ButtonReleased(..)
                )
            ) {
                state.pressed = false;
                state.press_pos = None;
                state.dragging = false;
                state.last_pos = None;
            }
            return;
        }

        // --- Post-children: clicks and keyboard ---
        // Only fire click/right-click if no child widget captured the event.
        match event {
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                if state.pressed && !state.dragging {
                    if let Some(ref on_click) = self.on_click {
                        if let Some(pos) = cursor.position() {
                            if let Some(message) = on_click(pos.x, pos.y) {
                                shell.publish(message);
                            }
                        }
                    }
                }
                state.pressed = false;
                state.press_pos = None;
                state.dragging = false;
                state.last_pos = None;
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Right)) => {
                if let Some(ref on_right_click) = self.on_right_click {
                    if let Some(pos) = cursor.position() {
                        if let Some(message) = on_right_click(pos.x, pos.y) {
                            shell.publish(message);
                        }
                    }
                }
            }
            Event::Keyboard(keyboard::Event::KeyPressed { key, repeat, .. }) => {
                if let Some(message) = (self.on_key_press)(key.clone(), *repeat) {
                    shell.publish(message);
                }
            }
            _ => {}
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: iced::mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: iced::mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> iced::mouse::Interaction {
        let state = tree.state.downcast_ref::<State>();
        if state.dragging {
            return iced::mouse::Interaction::Grabbing;
        }
        self.content
            .as_widget()
            .mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer)
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

fn distance(a: Point, b: Point) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

impl<'a, Message, Theme, Renderer> From<KeyListener<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: iced::advanced::Renderer + 'a,
{
    fn from(listener: KeyListener<'a, Message, Theme, Renderer>) -> Self {
        Element::new(listener)
    }
}
