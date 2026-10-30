//! Private notification intent, send, update, and approval-resolution facade. Providers do not acquire workflow decision authority.

mod approval;
mod delivery;
mod updates;

pub(in crate::durable) use delivery::notification_request_all_failed;
