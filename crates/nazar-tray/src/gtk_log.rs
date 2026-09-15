//! One GTK assertion, out of the log it does not belong in.
//!
//! Running the tray on a GNOME Wayland session prints this, once per icon refresh, for as
//! long as the application is open:
//!
//! ```text
//! (nazar-tray:66001): Gtk-CRITICAL **: gtk_widget_get_scale_factor: assertion 'GTK_IS_WIDGET (widget)' failed
//! ```
//!
//! **It is not this program's call, and there is nothing in this program to fix.** The stack,
//! read under gdb with `G_DEBUG=fatal-criticals`, is six frames of somebody else's code:
//!
//! ```text
//! gtk_widget_get_scale_factor        libgtk-3.so.0
//! gtk_status_icon_update_image       libgtk-3.so.0
//! gtk_status_icon_set_from_file      libgtk-3.so.0
//! status_icon_changes                libappindicator3.so.1
//! fallback                           libappindicator3.so.1
//! fallback_timer_expire              libappindicator3.so.1
//! ```
//!
//! `fallback` is the word that explains it. When the desktop offers no
//! `org.kde.StatusNotifierWatcher` — which a stock GNOME does not, extension or no extension
//! — libappindicator gives up on the modern protocol after a timer and falls back to
//! `GtkStatusIcon`, the deprecated X11 system-tray widget. `GtkStatusIcon` builds its image
//! widget only on an X11 display, so on Wayland it has none, and every icon it is handed
//! asks a null pointer for its scale factor. Measured both ways on the same machine: five
//! criticals over a `--demo-cross` run, and zero for the identical run under
//! `GDK_BACKEND=x11`.
//!
//! So the choice is not "fix it or leave it" but "print it or do not", and the argument for
//! not printing it is that it is **unactionable and unbounded**: it says nothing a user or a
//! maintainer can act on, it names a function this repository does not call, and it arrives
//! again every time the bead is redrawn. A user running the tray from a terminal to see what
//! it is doing would have their own output buried by it.
//!
//! **What is filtered is one message, not a level and not a library.** Any other Gtk
//! critical — including a real one from code here — goes to `g_log_writer_default`, which is
//! exactly where GLib would have sent it, journald integration and all. The filter is a
//! string match rather than a flag, so the day GTK or libappindicator fixes this, the match
//! simply stops matching and nothing else changes.
//!
//! The *complete* answer is not to take the fallback at all, which means knowing whether the
//! desktop has a watcher before registering a tray icon and telling the user when it does
//! not. That is T-WP-L2's question and its decision is not this package's to make.

use std::ffi::{CStr, c_char, c_int, c_uint, c_void};
use std::sync::Once;

/// `GLogField`, as `gmessages.h` declares it: a key, a value that is usually a string, and a
/// length that is `-1` when the value is NUL-terminated.
#[repr(C)]
struct LogField {
    key: *const c_char,
    value: *const c_void,
    length: isize,
}

/// `G_LOG_WRITER_HANDLED` — this message is dealt with, write nothing.
const HANDLED: c_int = 1;

unsafe extern "C" {
    /// Install the process-wide structured-log writer. GLib allows exactly one, and calling
    /// this twice is a programmer error, which is what the [`Once`] in [`install`] is for.
    fn g_log_set_writer_func(
        func: Option<unsafe extern "C" fn(c_uint, *const LogField, usize, *mut c_void) -> c_int>,
        user_data: *mut c_void,
        user_data_free: Option<unsafe extern "C" fn(*mut c_void)>,
    );

    /// GLib's own writer: the one that would have run if nothing here existed. Everything
    /// this module does not recognise is handed straight to it.
    fn g_log_writer_default(
        log_level: c_uint,
        fields: *const LogField,
        n_fields: usize,
        user_data: *mut c_void,
    ) -> c_int;
}

/// Filter the one known message out of GLib's log, once per process.
///
/// Call before anything else in `main`: a writer installed after a message has been logged
/// does not un-log it, and GLib accepts only the first one.
pub fn install() {
    static INSTALLED: Once = Once::new();
    // SAFETY: called exactly once, with a function pointer of the declared signature, no
    // user data and no destructor for the user data there is none of.
    INSTALLED
        .call_once(|| unsafe { g_log_set_writer_func(Some(writer), std::ptr::null_mut(), None) });
}

/// The writer GLib calls for every structured log message in this process.
unsafe extern "C" fn writer(
    log_level: c_uint,
    fields: *const LogField,
    n_fields: usize,
    user_data: *mut c_void,
) -> c_int {
    // SAFETY: GLib passes an array of `n_fields` initialised `GLogField`s that outlives the
    // call, which is precisely what `read_field` and `from_raw_parts` are given.
    let (domain, message) = unsafe { domain_and_message(fields, n_fields) };
    if is_the_status_icon_fallback_assertion(domain.as_deref(), message.as_deref()) {
        return HANDLED;
    }
    // SAFETY: the arguments are the ones GLib handed in, forwarded unchanged to the writer
    // it would have called itself.
    unsafe { g_log_writer_default(log_level, fields, n_fields, user_data) }
}

/// `GLIB_DOMAIN` and `MESSAGE` out of a field array, as far as they are readable text.
///
/// # Safety
///
/// `fields` must point to `n_fields` initialised `GLogField`s that stay alive for the call.
unsafe fn domain_and_message(
    fields: *const LogField,
    n_fields: usize,
) -> (Option<String>, Option<String>) {
    if fields.is_null() {
        return (None, None);
    }
    let mut domain = None;
    let mut message = None;
    // SAFETY: the caller's guarantee.
    for field in unsafe { std::slice::from_raw_parts(fields, n_fields) } {
        // SAFETY: a field's key is a NUL-terminated static string in every GLib that has
        // this API.
        let key = unsafe { text(field.key.cast(), -1) };
        match key.as_deref() {
            // SAFETY: the value is the field's own, read to its own length.
            Some("GLIB_DOMAIN") => domain = unsafe { text(field.value.cast(), field.length) },
            Some("MESSAGE") => message = unsafe { text(field.value.cast(), field.length) },
            _ => {}
        }
    }
    (domain, message)
}

/// A field value as a `String`: NUL-terminated when `length` is negative, counted otherwise.
///
/// A value that is not text at all — GLib allows arbitrary bytes — is `None` rather than a
/// guess, and a message that cannot be read is a message that cannot be filtered.
///
/// # Safety
///
/// `value` must be null, or point to `length` readable bytes, or to a NUL-terminated string
/// when `length` is negative.
unsafe fn text(value: *const c_char, length: isize) -> Option<String> {
    if value.is_null() {
        return None;
    }
    let bytes = if length < 0 {
        // SAFETY: the caller's guarantee that this is NUL-terminated.
        unsafe { CStr::from_ptr(value) }.to_bytes()
    } else {
        // SAFETY: the caller's guarantee that `length` bytes are readable.
        unsafe { std::slice::from_raw_parts(value.cast::<u8>(), length as usize) }
    };
    std::str::from_utf8(bytes).ok().map(ToOwned::to_owned)
}

/// Whether a log message is the GTK assertion libappindicator's Wayland fallback produces.
///
/// Narrow on purpose. The domain pins it to GTK, and the two fragments pin it to one
/// assertion in one function: a Gtk critical about anything else, from anywhere else, is not
/// this and is printed.
fn is_the_status_icon_fallback_assertion(domain: Option<&str>, message: Option<&str>) -> bool {
    let (Some(domain), Some(message)) = (domain, message) else {
        return false;
    };
    domain == "Gtk"
        && message.contains("gtk_widget_get_scale_factor")
        && message.contains("GTK_IS_WIDGET")
}

#[cfg(test)]
mod tests {
    use super::is_the_status_icon_fallback_assertion;

    /// The message as it actually arrives, copied from a run on this machine.
    const THE_MESSAGE: &str =
        "gtk_widget_get_scale_factor: assertion 'GTK_IS_WIDGET (widget)' failed";

    #[test]
    fn the_wayland_fallback_assertion_is_recognised() {
        assert!(is_the_status_icon_fallback_assertion(
            Some("Gtk"),
            Some(THE_MESSAGE)
        ));
    }

    /// The filter is one message, not a level and not a library: everything else still goes
    /// to GLib's own writer, including criticals from GTK itself.
    #[test]
    fn nothing_else_is_swallowed() {
        for (domain, message) in [
            (Some("Gtk"), Some("gtk_widget_show: assertion failed")),
            (
                Some("Gtk"),
                Some("gtk_widget_get_scale_factor: something else entirely"),
            ),
            (Some("GLib-GIO"), Some(THE_MESSAGE)),
            (Some("Gdk"), Some(THE_MESSAGE)),
            (None, Some(THE_MESSAGE)),
            (Some("Gtk"), None),
            (None, None),
        ] {
            assert!(
                !is_the_status_icon_fallback_assertion(domain, message),
                "{domain:?} {message:?}"
            );
        }
    }
}
