// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 nicococo

//! Build a minimal RFC 5322 `.eml` file from an [`EmailMessage`] so
//! `o` (open) has something to hand the OS's default handler for.
//!
//! IMAP messages never carry a `web_url` (no provider gives a canonical
//! web link for a raw IMAP message — see the doc comment on that
//! field; a Gmail-specific web-search deep link was tried and reverted,
//! since none of the account-selection tricks reliably land on the
//! right signed-in account without an extra manual step), so "open in
//! mail client" can't just launch a browser URL. Writing the message
//! out as a standalone `.eml` and opening *that* file lets the OS's
//! file-type association do the rest — Thunderbird, Apple Mail,
//! Outlook desktop, and most other clients register themselves as the
//! `.eml` handler.
//!
//! This is a read-only preview, not a faithful copy of the original
//! wire message — headers this widget doesn't track (raw `Message-ID`,
//! `To`, MIME structure of a multipart original) are synthesized or
//! omitted. Good enough to view the message; not suitable for
//! forwarding as a byte-identical copy of what the server holds.

use super::provider::EmailMessage;

/// Render `msg` as a complete `.eml` document (headers + blank line +
/// plain-text body). Pure string building — no I/O, so it's cheap to
/// unit test independent of the temp-file plumbing in `mod.rs`.
pub fn build(msg: &EmailMessage) -> String {
    let from = match &msg.from_name {
        Some(name) if !name.is_empty() => format!("{} <{}>", quote_display_name(name), msg.from_address),
        _ => msg.from_address.clone(),
    };
    let message_id = format!("<{}@docket.local>", sanitize_id(&msg.id));
    let date = msg.received.to_rfc2822();

    format!(
        "From: {from}\r\n\
         Subject: {subject}\r\n\
         Date: {date}\r\n\
         Message-ID: {message_id}\r\n\
         MIME-Version: 1.0\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\
         Content-Transfer-Encoding: 8bit\r\n\
         \r\n\
         {body}\r\n",
        subject = msg.subject,
        body = msg.plain_body,
    )
}

/// Wrap a display name in double quotes and escape any embedded quote
/// or backslash — the minimum RFC 5322 `quoted-string` handling needed
/// for a display name that might contain a comma or other special
/// char (common in "Last, First" senders).
fn quote_display_name(name: &str) -> String {
    let escaped = name.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// Filesystem/Message-ID-safe form of a provider message id (an IMAP
/// UID, stringified — normally already just digits, but sanitize
/// defensively since it round-trips through a cache file). Also used
/// by `mod.rs` to build the temp `.eml` filename from the account
/// label + message id.
pub(super) fn sanitize_id(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn sample() -> EmailMessage {
        EmailMessage {
            id: "42".into(),
            folder: "INBOX".into(),
            from_name: Some("Ada Lovelace".into()),
            from_address: "ada@example.com".into(),
            subject: "Re: the analytical engine".into(),
            received: chrono::Local
                .with_ymd_and_hms(2026, 8, 28, 12, 30, 0)
                .unwrap(),
            server_unread: false,
            plain_body: "Line one.\nLine two.".into(),
            web_url: None,
            account: "personal".into(),
            imap_uid: Some(42),
        }
    }

    #[test]
    fn build_includes_core_headers_and_body() {
        let out = build(&sample());
        assert!(out.starts_with("From: \"Ada Lovelace\" <ada@example.com>\r\n"));
        assert!(out.contains("Subject: Re: the analytical engine\r\n"));
        assert!(out.contains("Message-ID: <42@docket.local>\r\n"));
        assert!(out.contains("Content-Type: text/plain; charset=utf-8\r\n"));
        assert!(out.contains("\r\n\r\nLine one.\nLine two.\r\n"));
    }

    #[test]
    fn build_falls_back_to_bare_address_without_a_display_name() {
        let mut msg = sample();
        msg.from_name = None;
        let out = build(&msg);
        assert!(out.starts_with("From: ada@example.com\r\n"));
    }

    #[test]
    fn build_escapes_quotes_in_display_name() {
        let mut msg = sample();
        msg.from_name = Some("The \"Analytical\" Engine".into());
        let out = build(&msg);
        assert!(out.starts_with("From: \"The \\\"Analytical\\\" Engine\" <ada@example.com>\r\n"));
    }

    #[test]
    fn sanitize_id_strips_unsafe_chars() {
        assert_eq!(sanitize_id("42"), "42");
        assert_eq!(sanitize_id("uid/with:odd chars"), "uid_with_odd_chars");
    }
}
