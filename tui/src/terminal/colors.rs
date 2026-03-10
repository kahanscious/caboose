//! Map vt100 colors to ratatui colors.

use ratatui::style::Color;

/// Convert a vt100 color to its ratatui equivalent.
pub fn to_ratatui_color(color: vt100::Color) -> Color {
    match color {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

/// Convert vt100 cell attributes to a ratatui Style.
pub fn cell_style(cell: &vt100::Cell) -> ratatui::style::Style {
    let mut style = ratatui::style::Style::default()
        .fg(to_ratatui_color(cell.fgcolor()))
        .bg(to_ratatui_color(cell.bgcolor()));

    if cell.bold() {
        style = style.add_modifier(ratatui::style::Modifier::BOLD);
    }
    if cell.italic() {
        style = style.add_modifier(ratatui::style::Modifier::ITALIC);
    }
    if cell.underline() {
        style = style.add_modifier(ratatui::style::Modifier::UNDERLINED);
    }
    if cell.inverse() {
        style = style.add_modifier(ratatui::style::Modifier::REVERSED);
    }

    style
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_color_maps_to_reset() {
        assert_eq!(to_ratatui_color(vt100::Color::Default), Color::Reset);
    }

    #[test]
    fn indexed_color_maps_correctly() {
        assert_eq!(to_ratatui_color(vt100::Color::Idx(1)), Color::Indexed(1));
        assert_eq!(
            to_ratatui_color(vt100::Color::Idx(255)),
            Color::Indexed(255)
        );
    }

    #[test]
    fn rgb_color_maps_correctly() {
        assert_eq!(
            to_ratatui_color(vt100::Color::Rgb(212, 87, 42)),
            Color::Rgb(212, 87, 42)
        );
    }
}
