//! Keep the start's control and producer at the same priority as priming.
use std::{marker::PhantomData, rc::Rc};

/// Temporarily requests interactive scheduling for start publication.
/// Restores the creating thread's scheduling class when dropped.
#[doc(hidden)]
pub struct StartPriority {
    #[cfg(target_os = "macos")]
    previous: Option<(libc::qos_class_t, i32)>,
    // A priority guard must stay on its creating thread.
    _thread: PhantomData<Rc<()>>,
}
impl StartPriority {
    pub fn new() -> Self {
        #[cfg(target_os = "macos")]
        {
            let mut class = libc::qos_class_t::QOS_CLASS_UNSPECIFIED;
            let mut relative = 0;
            // SAFETY: valid output pointers; the calls affect this thread.
            let previous = unsafe {
                if libc::pthread_get_qos_class_np(libc::pthread_self(), &mut class, &mut relative)
                    != 0
                    || matches!(class, libc::qos_class_t::QOS_CLASS_UNSPECIFIED)
                {
                    class = libc::qos_class_t::QOS_CLASS_DEFAULT;
                    relative = 0;
                }
                (libc::pthread_set_qos_class_self_np(
                    libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE,
                    0,
                ) == 0)
                    .then_some((class, relative))
            };
            Self {
                previous,
                _thread: PhantomData,
            }
        }
        #[cfg(not(target_os = "macos"))]
        Self {
            _thread: PhantomData,
        }
    }
}
impl Default for StartPriority {
    fn default() -> Self {
        Self::new()
    }
}
impl Drop for StartPriority {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        if let Some((class, relative)) = self.previous {
            // SAFETY: the guard cannot move to another thread.
            unsafe {
                libc::pthread_set_qos_class_self_np(class, relative);
            }
        }
    }
}
#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    fn class() -> (u32, i32) {
        let mut class = libc::qos_class_t::QOS_CLASS_UNSPECIFIED;
        let mut relative = 0;
        unsafe {
            assert_eq!(
                libc::pthread_get_qos_class_np(libc::pthread_self(), &mut class, &mut relative),
                0
            );
        }
        if matches!(class, libc::qos_class_t::QOS_CLASS_UNSPECIFIED) {
            (libc::qos_class_t::QOS_CLASS_DEFAULT as u32, 0)
        } else {
            (class as u32, relative)
        }
    }
    #[test]
    fn nested_start_priority_restores_the_callers_class() {
        let before = class();
        {
            let _outer = StartPriority::new();
            assert_eq!(
                class().0,
                libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE as u32
            );
            {
                let _inner = StartPriority::new();
            }
            assert_eq!(
                class().0,
                libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE as u32
            );
        }
        assert_eq!(class(), before);
    }
}
