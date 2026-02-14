//! A transparent wrapper widget that intercepts keyboard events synchronously.
//! Unlike subscription-based keyboard handling, messages are produced in the
//! same frame as the event — no async executor delay.
//!
//! Also supports scroll interception for zoom gestures and mouse drag for panning.

use iced::advanced::layout;
use iced::advanced::overlay;
use iced::advanced::renderer;
use iced::advanced::widget::{Operation, Tree};
use iced::advanced::widget::tree::Tag;
use iced::advanced::{Clipboard, Layout, Shell, Widget};
use iced::{keyboard, mouse, Element, Event, Length, Point, Rectangle, Size, Vector};

#[derive(Debug, Default)]
struct State {
    dragging: bool,
    last_pos: Option<Point>,
}

pub struct KeyListener<'a, Message, Theme = iced::Theme, Renderer = iced::Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
    on_key_press: Box<dyn Fn(keyboard::Key, bool) -> Option<Message> + 'a>,
    /// Called on scroll events with (delta, cursor_x, cursor_y).
    on_scroll: Option<Box<dyn Fn(f32, f32, f32) -> Option<Message> + 'a>>,
    /// Called on mouse drag with (dx, dy). Return Some to consume the event.
    on_drag: Option<Box<dyn Fn(f32, f32) -> Option<Message> + 'a>>,
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

        // --- Mouse drag handling (before children) ---
        if let Some(ref on_drag) = self.on_drag {
            match event {
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                    if let Some(pos) = cursor.position() {
                        state.dragging = true;
                        state.last_pos = Some(pos);
                    }
                }
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                    state.dragging = false;
                    state.last_pos = None;
                }
                Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                    if state.dragging {
                        if let (Some(last), Some(pos)) = (state.last_pos, cursor.position()) {
                            let dx = pos.x - last.x;
                            let dy = pos.y - last.y;
                            state.last_pos = Some(pos);
                            if dx.abs() > 0.5 || dy.abs() > 0.5 {
                                if let Some(message) = on_drag(dx, dy) {
                                    shell.publish(message);
                                    return;
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
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

        // Pass event to children
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
            return;
        }

        if let Event::Keyboard(keyboard::Event::KeyPressed { key, repeat, .. }) = event {
            if let Some(message) = (self.on_key_press)(key.clone(), *repeat) {
                shell.publish(message);
            }
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
