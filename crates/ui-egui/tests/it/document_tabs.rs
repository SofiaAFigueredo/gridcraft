//! The workbook tab strip: shown from two open workbooks on, clicks switch, close and add.
use egui::{Event, PointerButton, Pos2, accesskit::Role};
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use gridcraft_engine::Session;
use serde_json::json;

use crate::common::Harness;
use crate::support;

fn books(n: usize) -> Harness {
    let mut s = Session::new();
    for _ in 0..n {
        s.new_workbook();
    }
    support::harness(s, 6)
}

fn tab_titles(h: &Harness) -> Vec<String> {
    h.query_all_by_role(Role::Tab).filter_map(|n| n.accesskit_node().label()).collect()
}

/// The × of a tab: 15 px in from the tab's right edge, on its row (see `document_tabs`).
fn close_button(h: &Harness, title: &str) -> Pos2 {
    let r = h.get_by_role_and_label(Role::Tab, title).rect();
    Pos2::new(r.right() - 15.0, r.center().y + 1.0)
}

fn move_to(h: &mut Harness, at: Pos2) {
    h.input_mut().events.push(Event::PointerMoved(at));
    h.run_steps(4);
}

fn click_at(h: &mut Harness, at: Pos2) {
    move_to(h, at);
    h.input_mut().events.push(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.step();
    h.input_mut().events.push(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(8);
}

#[test]
fn a_single_workbook_shows_no_tab_strip() {
    assert!(tab_titles(&books(1)).is_empty());
}

#[test]
fn every_open_workbook_gets_a_tab() {
    assert_eq!(tab_titles(&books(3)), ["Book1", "Book2", "Book3"]);
}

#[test]
fn clicking_a_tab_switches_workbooks() {
    let mut h = books(2);
    assert_eq!(h.state().session.active_index(), 1, "the newest workbook is active");
    h.get_by_role_and_label(Role::Tab, "Book1").click();
    h.run_steps(8);
    assert_eq!(h.state().session.active_index(), 0);
    h.get_by_role_and_label(Role::Tab, "Book2").click();
    h.run_steps(8);
    assert_eq!(h.state().session.active_index(), 1);
}

#[test]
fn the_close_button_closes_that_workbook_and_the_strip_goes_with_the_last_sibling() {
    let mut h = books(2);
    // Book1 is not the active tab: it becomes active, then closes.
    let x = close_button(&h, "Book1");
    click_at(&mut h, x);
    assert_eq!(h.state().session.documents().len(), 1);
    assert_eq!(h.state().session.documents()[0].title, "Book2");
    assert!(tab_titles(&h).is_empty(), "one workbook left: no strip");
}

#[test]
fn a_middle_click_closes_a_tab() {
    let mut h = books(2);
    h.get_by_role_and_label(Role::Tab, "Book2").click_button(PointerButton::Middle);
    h.run_steps(8);
    assert_eq!(h.state().session.documents().len(), 1);
    assert_eq!(h.state().session.documents()[0].title, "Book1");
}

#[test]
fn closing_a_workbook_with_unsaved_changes_asks_first() {
    let mut h = books(2);
    h.state_mut().run("cell.set", json!({"cell": "A1", "input": "7"})).unwrap();
    h.run_steps(4);
    let x = close_button(&h, "Book2");
    click_at(&mut h, x);
    assert_eq!(h.state().session.documents().len(), 2, "still open");
    assert!(h.state().dialog.as_ref().is_some_and(|d| d.name() == "saveChanges"), "asks about the changes");
}

#[test]
fn the_new_button_adds_a_workbook_tab() {
    let mut h = books(2);
    let plus = h.get_by_role_and_label(Role::Button, "New Workbook").rect().center();
    click_at(&mut h, plus);
    assert_eq!(tab_titles(&h), ["Book1", "Book2", "Book3"]);
    assert_eq!(h.state().session.active_index(), 2);
}

#[test]
fn hidden_windows_have_no_tab() {
    let mut h = books(3);
    h.state_mut().run("view.hideWindow", json!({})).unwrap();
    h.run_steps(6);
    assert_eq!(tab_titles(&h), ["Book1", "Book2"]);
}
