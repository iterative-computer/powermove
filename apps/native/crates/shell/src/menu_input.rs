//! Decide which mouse presses belong to an open menu.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    Menu,
    Trigger,
    OtherTrigger,
    Outside,
    WindowChrome,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Response {
    Pass,
    DismissAndPass,
    DismissAndConsume,
}

pub fn press_response(press: Press) -> Response {
    match press {
        Press::Menu => Response::Pass,
        Press::OtherTrigger | Press::WindowChrome => Response::DismissAndPass,
        Press::Trigger | Press::Outside => Response::DismissAndConsume,
    }
}

/// Activating a closed menu or a different trigger opens the requested menu.
pub fn opens_after_activation(current_trigger: bool) -> bool {
    !current_trigger
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressing_the_open_trigger_dismisses_without_replaying_the_click() {
        assert_eq!(press_response(Press::Trigger), Response::DismissAndConsume);
    }

    #[test]
    fn outside_press_dismisses_and_does_not_activate_the_target() {
        assert_eq!(press_response(Press::Outside), Response::DismissAndConsume);
    }

    #[test]
    fn another_menu_opens_on_the_same_click() {
        assert_eq!(
            press_response(Press::OtherTrigger),
            Response::DismissAndPass
        );
        assert!(opens_after_activation(false));
    }

    #[test]
    fn window_controls_and_menu_rows_keep_their_clicks() {
        assert_eq!(press_response(Press::Menu), Response::Pass);
        assert_eq!(
            press_response(Press::WindowChrome),
            Response::DismissAndPass
        );
    }

    #[test]
    fn keyboard_activation_of_the_same_trigger_closes_the_menu() {
        assert!(!opens_after_activation(true));
        assert!(opens_after_activation(false));
    }
}
