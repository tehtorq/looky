//! A transparent wrapper widget that intercepts keyboard events synchronously.
//! Unlike subscription-based keyboard handling, messages are produced in the
//! same frame as the event — no async executor delay.
//!
//! Also supports Ctrl+scroll interception for zoom gestures (including
//! trackpad pinch-to-zoom which maps to Ctrl+scroll on Linux).

use iced::advanced::layout;
use iced::advanced::overlay;
use iced::advanced::renderer;
use iced::advanced::widget::{Operation, Tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget};
use iced::{keyboard, mouse, Element, Event, Length, Rectangle, Size, Vector};

pub struct KeyListener<'a, Message, Theme = iced::Theme, Renderer = iced::Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
    on_key_press: Box<dyn Fn(keyboard::Key, bool) -> Option<Message> + 'a>,
    /// Called on scroll events. If it returns Some, the event is intercepted
    /// (children like scrollable won't see it). If None, the event passes through.
    on_scroll: Option<Box<dyn Fn(f32) -> Option<Message> + 'a>>,
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
        }
    }

    pub fn on_scroll(
        mut self,
        f: impl Fn(f32) -> Option<Message> + 'a,
    ) -> Self {
        self.on_scroll = Some(Box::new(f));
        self
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for KeyListener<'_, Message, Theme, Renderer>
where
    Renderer: iced::advanced::Renderer,
{
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
        // Check scroll events BEFORE children so we can intercept them.
        // If the callback returns Some, we consume the event (the scrollable
        // won't see it). If None, we let it pass through to children.
        if let Event::Mouse(mouse::Event::WheelScrolled { delta }) = event {
            if let Some(ref on_scroll) = self.on_scroll {
                // Normalize to "line" units (~1.0 per mouse wheel click).
                // Trackpad pixel events use /40 so a typical swipe
                // (≈200-400px total over many events) maps to several "lines".
                let y = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => *y,
                    mouse::ScrollDelta::Pixels { y, .. } => *y / 40.0,
                };
                if y.abs() > 0.001 {
                    if let Some(message) = on_scroll(y) {
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
