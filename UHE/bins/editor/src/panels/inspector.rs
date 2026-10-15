//! The right panel: the components of the selected entity.
//!
//! The C++ `SceneHierarchyPanel::DrawComponents` walked the selected entity's
//! components, drew each as a collapsing header, and gave every field a control —
//! `DragFloat` with coloured X/Y/Z buttons for the vectors, `ColorEdit4` for
//! colors, checkboxes for the booleans. `mirror::inspector` owns the component
//! table and the read/write closures; this draws its `Property` list and writes
//! back through `Editor::set_property`.
//!
//! Three controls live here rather than in `mirror`, because they need a toolkit:
//! booleans toggle on click, an enum advances to its next variant, and a color
//! shows its swatch. A float has no editor: gpui has no drag-number widget, so the
//! value is shown the way the C++ formatted it and is not editable.

use gpui::{div, prelude::*, px, AnyElement, ClickEvent, Context, IntoElement, Rgba, SharedString};
use meats::{Entity, Scene};
use mirror::inspector::{Property, PropertyKind, PropertyValue};
use mirror::Editor;

use crate::app::EditorApp;
use crate::panels::{body, button, dim_label, ROW_HEIGHT};
use crate::theme::EditorTheme;

pub fn panel(
    editor: &Editor,
    scene: &Scene,
    theme: &EditorTheme,
    cx: &mut Context<EditorApp>,
) -> impl IntoElement {
    let panel = body().bg(theme.panel).child(
        div()
            .flex_none()
            .h(px(26.0))
            .px_2()
            .flex()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .text_xs()
            .text_color(theme.text_dim)
            .child("PROPERTIES"),
    );

    let Some(entity) = editor.selected_entity() else {
        return panel.child(dim_label("Select an entity to view properties", theme));
    };

    let name = editor
        .rows(scene)
        .into_iter()
        .find(|row| row.entity == entity)
        .map(|row| row.name)
        .unwrap_or_else(|| "-".to_string());

    let properties = editor.properties(scene);
    let mut rows: Vec<AnyElement> = Vec::with_capacity(properties.len() + 1);
    let mut component = "";
    for (index, property) in properties.iter().enumerate() {
        if property.component != component {
            component = property.component;
            rows.push(component_header(component, theme).into_any_element());
        }
        rows.push(property_row(index, property, scene, entity, theme, cx).into_any_element());
    }

    panel
        .child(
            div()
                .flex_none()
                .h(px(28.0))
                .px_2()
                .flex()
                .items_center()
                .justify_between()
                .gap_1()
                .text_sm()
                .text_color(theme.text)
                .child(div().flex_1().min_w_0().truncate().child(name))
                .child(button(
                    "inspector-unparent",
                    "Root",
                    false,
                    theme,
                    cx,
                    move |this, _window, cx| {
                        let scene = &mut this.engine.engine_mut().scene;
                        this.editor.reparent(scene, entity, None);
                        cx.notify();
                    },
                ))
                .child(button(
                    "inspector-delete",
                    "Delete",
                    false,
                    theme,
                    cx,
                    move |this, _window, cx| {
                        let scene = &mut this.engine.engine_mut().scene;
                        this.editor.destroy_entity(scene, entity);
                        cx.notify();
                    },
                )),
        )
        .child(
            div()
                .id("inspector-list")
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .py_1()
                .overflow_y_scroll()
                .children(rows),
        )
}

fn component_header(component: &str, theme: &EditorTheme) -> impl IntoElement {
    div()
        .flex_none()
        .px_2()
        .pt_2()
        .pb_1()
        .text_xs()
        .text_color(theme.accent)
        .child(SharedString::from(component.to_string()))
}

fn property_row(
    index: usize,
    property: &Property,
    scene: &Scene,
    entity: Entity,
    theme: &EditorTheme,
    cx: &mut Context<EditorApp>,
) -> impl IntoElement {
    let value = property.value(scene, entity);
    let component = property.component;
    let name = property.name;
    let hover = theme.row_hover;
    let toggled = matches!(value, PropertyValue::Bool(true));

    let row = div()
        .id(("inspector-property", index))
        .flex_none()
        .h(px(ROW_HEIGHT))
        .px_2()
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .text_sm()
        .child(
            div()
                .flex_none()
                .text_color(theme.text_dim)
                .child(SharedString::from(name)),
        );

    match &property.kind {
        PropertyKind::Bool => row
            .cursor_pointer()
            .hover(move |style| style.bg(hover))
            .on_click(cx.listener(move |this, _event: &ClickEvent, _window, cx| {
                apply(this, component, name, PropertyValue::Bool(!toggled));
                cx.notify();
            }))
            .child(value_label(format_value(&value), theme)),
        PropertyKind::Enum(variants) => match next_variant(*variants, &value) {
            Some(next) => row
                .cursor_pointer()
                .hover(move |style| style.bg(hover))
                .on_click(cx.listener(move |this, _event: &ClickEvent, _window, cx| {
                    apply(this, component, name, PropertyValue::Int(next));
                    cx.notify();
                }))
                .child(value_label(enum_label(*variants, &value), theme)),
            None => row.child(value_label(format_value(&value), theme)),
        },
        _ => match &value {
            PropertyValue::Color(channels) => {
                row.child(value_label(format_value(&value), theme))
                    .child(swatch(*channels))
            }
            _ => row.child(value_label(format_value(&value), theme)),
        },
    }
}

/// Writes one property back through the model.
///
/// `Property` cannot be captured by a callback that outlives this frame: its
/// accessors are owned closures rebuilt per frame. The handle that survives is the
/// `(component, name)` pair, so the property is looked up again when the click
/// arrives.
fn apply(this: &mut EditorApp, component: &'static str, name: &'static str, value: PropertyValue) {
    let scene = &mut this.engine.engine_mut().scene;
    let properties = this.editor.properties(scene);
    if let Some(property) = properties
        .iter()
        .find(|property| property.component == component && property.name == name)
    {
        this.editor.set_property(scene, property, value);
    }
}

fn value_label(text: String, theme: &EditorTheme) -> impl IntoElement {
    div()
        .flex_none()
        .text_color(theme.text)
        .child(SharedString::from(text))
}

fn swatch(channels: [f32; 4]) -> impl IntoElement {
    div()
        .flex_none()
        .w(px(12.0))
        .h(px(12.0))
        .rounded_sm()
        .border_1()
        .border_color(Rgba {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 0.2,
        })
        .bg(Rgba {
            r: channels[0].clamp(0.0, 1.0),
            g: channels[1].clamp(0.0, 1.0),
            b: channels[2].clamp(0.0, 1.0),
            a: channels[3].clamp(0.0, 1.0),
        })
}

/// The variant an enum advances to, when the value names one.
pub fn next_variant(variants: &'static [&'static str], value: &PropertyValue) -> Option<i32> {
    match value {
        PropertyValue::Int(current) if !variants.is_empty() => {
            Some((current + 1).rem_euclid(variants.len() as i32))
        }
        _ => None,
    }
}

/// An enum's current variant by name, falling back to the raw value when it names
/// no variant.
pub fn enum_label(variants: &'static [&'static str], value: &PropertyValue) -> String {
    if let PropertyValue::Int(index) = value {
        if let Some(name) = variants.get(*index as usize) {
            return (*name).to_string();
        }
    }
    format_value(value)
}

/// The value as text, in the precision the C++ controls used: `%.2f` for floats,
/// the raw integer, `true`/`false`.
pub fn format_value(value: &PropertyValue) -> String {
    match value {
        PropertyValue::Bool(value) => {
            if *value {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
        PropertyValue::Int(value) => value.to_string(),
        PropertyValue::Float(value) => format!("{value:.2}"),
        PropertyValue::Vec2(value) => format!("{:.2}, {:.2}", value[0], value[1]),
        PropertyValue::Vec3(value) => format!("{:.2}, {:.2}, {:.2}", value[0], value[1], value[2]),
        PropertyValue::Color(value) => format!(
            "{:.2}, {:.2}, {:.2}, {:.2}",
            value[0], value[1], value[2], value[3]
        ),
        PropertyValue::Text(value) => value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static BODY: &[&str] = &["Static", "Kinematic", "Dynamic"];

    #[test]
    fn scalars_keep_two_decimals() {
        assert_eq!(format_value(&PropertyValue::Float(1.0)), "1.00");
        assert_eq!(format_value(&PropertyValue::Float(-0.5)), "-0.50");
        assert_eq!(format_value(&PropertyValue::Int(-3)), "-3");
        assert_eq!(format_value(&PropertyValue::Bool(true)), "true");
        assert_eq!(format_value(&PropertyValue::Bool(false)), "false");
    }

    #[test]
    fn vectors_are_comma_separated() {
        assert_eq!(format_value(&PropertyValue::Vec3([1.0, 2.5, -3.25])), "1.00, 2.50, -3.25");
        assert_eq!(format_value(&PropertyValue::Vec2([0.0, 0.0])), "0.00, 0.00");
        assert_eq!(
            format_value(&PropertyValue::Color([1.0, 1.0, 1.0, 0.5])),
            "1.00, 1.00, 1.00, 0.50"
        );
    }

    #[test]
    fn text_passes_through() {
        assert_eq!(format_value(&PropertyValue::Text("player".into())), "player");
    }

    #[test]
    fn enums_show_and_advance_their_variant() {
        assert_eq!(enum_label(BODY, &PropertyValue::Int(2)), "Dynamic");
        assert_eq!(enum_label(BODY, &PropertyValue::Int(9)), "9");
        assert_eq!(enum_label(BODY, &PropertyValue::Float(1.5)), "1.50");

        assert_eq!(next_variant(BODY, &PropertyValue::Int(0)), Some(1));
        assert_eq!(next_variant(BODY, &PropertyValue::Int(2)), Some(0));
        assert_eq!(next_variant(BODY, &PropertyValue::Text("x".into())), None);
        assert_eq!(next_variant(&[], &PropertyValue::Int(0)), None);
    }
}
