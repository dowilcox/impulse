//! test
use gpui_kit::Action;

#[derive(Clone, PartialEq, Debug, serde::Deserialize, Action)]
#[action(namespace = impulse, no_json)]
pub struct RunCommand {
    pub id: String,
}
