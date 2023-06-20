use cfg_if::cfg_if;
use std::time::Duration;
use worker::Delay;

cfg_if! {
    // https://github.com/rustwasm/console_error_panic_hook#readme
    if #[cfg(feature = "console_error_panic_hook")] {
        extern crate console_error_panic_hook;
        pub use self::console_error_panic_hook::set_once as set_panic_hook;
    } else {
        #[inline]
        pub fn set_panic_hook() {}
    }
}

pub async fn delay(delay: u64) {
    let delay: Delay = Duration::from_millis(delay).into();
    delay.await;
}
