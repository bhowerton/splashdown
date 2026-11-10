//! Links to video calls, as a calendar event carries them: in its URL, its
//! location or its notes. Found so a meeting that's about to start can be
//! joined with one click.
//!
//! An event's text is written by whoever sent the invitation, so a link is
//! only taken when it's `https://` and its host belongs to one of the
//! services listed here. Anything else is never opened.

/// The video-call services whose links are known: each host (and the hosts
/// under it, such as `us02web.zoom.us`), and what the service is called.
const SERVICES: &[(&str, &str)] = &[
    ("zoom.us", "Zoom"),
    ("zoomgov.com", "Zoom"),
    ("meet.google.com", "Google Meet"),
    ("teams.microsoft.com", "Teams"),
    ("teams.live.com", "Teams"),
    ("teams.cloud.microsoft", "Teams"),
    ("teams.microsoft.us", "Teams"),
    ("webex.com", "Webex"),
    ("gotomeeting.com", "GoTo Meeting"),
    ("meet.goto.com", "GoTo Meeting"),
    ("whereby.com", "Whereby"),
    ("meet.jit.si", "Jitsi Meet"),
];

/// The longest link that's taken: real ones are a few hundred characters.
const LONGEST: usize = 2048;

/// A link to a video call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallLink {
    /// The link, starting `https://` and its host in small letters.
    pub url: String,
    /// The service it's on: "Zoom", "Google Meet"…
    pub service: &'static str,
}

/// The first link to a known video-call service in `text`: an event's URL,
/// location and notes, or any text with links in it.
pub fn find(text: &str) -> Option<CallLink> {
    text.split(|c: char| c.is_whitespace() || c.is_control() || "<>\"'()[]".contains(c))
        .find_map(|word| {
            // The link is the word from its scheme on ("Join:https://…").
            // One inside another link isn't a link of its own.
            let colon = word.find("://")?;
            let scheme = word[..colon]
                .bytes()
                .rev()
                .take_while(u8::is_ascii_alphabetic)
                .count();
            let link = word[colon - scheme..].trim_end_matches(['.', ',', ';', ':', '!', '?']);
            // Notes that were a web page write "&" this way.
            known(&link.replace("&amp;", "&"))
        })
}

/// `link` as a link to a known video-call service, when it is one and
/// nothing else: `https://`, a host from the list or under one, with no
/// user name or port, and no characters a link doesn't have.
pub fn known(link: &str) -> Option<CallLink> {
    if link.len() > LONGEST
        || !link
            .chars()
            .all(|c| c.is_ascii_graphic() && !"<>\"'\\^`{|}".contains(c))
    {
        return None;
    }
    let rest = link
        .get(..8)
        .filter(|scheme| scheme.eq_ignore_ascii_case("https://"))
        .map(|_| &link[8..])?;
    let (host, rest) = rest.split_at(rest.find(['/', '?', '#']).unwrap_or(rest.len()));
    let host = host.to_ascii_lowercase();
    // Names of letters, digits and hyphens with dots between them, and
    // nothing else: "zoom.us@example.com" and "zoom.us:8443" aren't hosts.
    let name = |name: &str| {
        !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    };
    if !host.split('.').all(name) {
        return None;
    }
    let (_, service) = SERVICES.iter().find(|(known, _)| {
        host == *known
            || host
                .strip_suffix(known)
                .is_some_and(|under| under.ends_with('.'))
    })?;
    Some(CallLink {
        url: format!("https://{host}{rest}"),
        service,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Option<String> {
        find(text).map(|link| link.url)
    }

    #[test]
    fn links_to_known_services_are_taken() {
        for (link, service) in [
            ("https://zoom.us/j/5551234567?pwd=abc123", "Zoom"),
            ("https://us02web.zoom.us/j/5551234567", "Zoom"),
            ("https://example.zoomgov.com/j/5551234567", "Zoom"),
            ("https://meet.google.com/abc-defg-hij", "Google Meet"),
            (
                "https://teams.microsoft.com/l/meetup-join/19%3ameeting_abc%40thread.v2/0?context=%7b%22Tid%22%3a%22x%22%7d",
                "Teams",
            ),
            ("https://teams.live.com/meet/9555123456789", "Teams"),
            ("https://example.webex.com/meet/dana.lee", "Webex"),
            ("https://meet.goto.com/555123456", "GoTo Meeting"),
            ("https://whereby.com/pricing-review", "Whereby"),
            ("https://meet.jit.si/PricingReview#config.x=1", "Jitsi Meet"),
        ] {
            let found = known(link).unwrap_or_else(|| panic!("{link}"));
            assert_eq!(found.url, link);
            assert_eq!(found.service, service);
            assert_eq!(find(link), Some(found));
        }
        // The host is compared in small letters, and written so.
        assert_eq!(
            url("HTTPS://US02Web.Zoom.US/j/555?pwd=AbC").as_deref(),
            Some("https://us02web.zoom.us/j/555?pwd=AbC")
        );
        assert_eq!(url("https://zoom.us").as_deref(), Some("https://zoom.us"));
    }

    #[test]
    fn other_links_are_refused() {
        for link in [
            // Not https.
            "http://zoom.us/j/5551234567",
            "zoommtg://zoom.us/join?confno=5551234567",
            "zoom.us/j/5551234567",
            "file:///Applications/Zoom.app",
            "javascript://zoom.us/%0aalert(1)",
            // Hosts that only look like one from the list.
            "https://zoom.us.example.com/j/5551234567",
            "https://evilzoom.us/j/5551234567",
            "https://zoom.us.evil.example/j/1",
            "https://meet.google.com.example.com/abc-defg-hij",
            "https://xmeet.google.com/abc-defg-hij",
            "https://google.com/url?q=https://meet.google.com/abc",
            "https://example.com/zoom.us/j/1",
            "https://example.com/?next=https://zoom.us/j/1",
            "https://example.com#zoom.us",
            "https://zoom.us./j/1",
            "https://.zoom.us/j/1",
            // A user name, a port, or other characters where the host is.
            "https://zoom.us@example.com/j/1",
            "https://zoom.us:pass@example.com/j/1",
            "https://example.com\\@zoom.us/j/1",
            "https://zoom.us\\.example.com/j/1",
            "https://zoom.us:8443/j/1",
            "https://zoom.us%2f.example.com/j/1",
            // Letters that only look like "o".
            "https://z\u{43e}\u{43e}m.us/j/1",
            "https:///zoom.us/j/1",
            "https://",
            "",
        ] {
            assert_eq!(known(link), None, "{link}");
            assert_eq!(find(link), None, "{link}");
        }
        // Nothing with a space or a line break in it is one link.
        assert_eq!(known("https://zoom.us/j/1 https://example.com"), None);
        assert_eq!(known("https://zoom.us/j/1\nx"), None);
        assert_eq!(
            known(&format!("https://zoom.us/j/{}", "1".repeat(LONGEST))),
            None
        );
    }

    #[test]
    fn a_link_is_found_inside_longer_text() {
        // An event's URL, location and notes, one after the other: the
        // first link that's known is the one.
        assert_eq!(
            url("https://example.com/agenda Room 4 / https://us02web.zoom.us/j/555?pwd=abc Agenda: https://meet.google.com/abc-defg-hij").as_deref(),
            Some("https://us02web.zoom.us/j/555?pwd=abc")
        );
        // As invitations write them.
        assert_eq!(
            url("Dana Lee is inviting you to a meeting.\n\nJoin Zoom Meeting\nhttps://zoom.us/j/5551234567?pwd=abc\n\nMeeting ID: 555 123 4567").as_deref(),
            Some("https://zoom.us/j/5551234567?pwd=abc")
        );
        assert_eq!(
            url("Join the meeting now<https://teams.microsoft.com/l/meetup-join/abc/0?context=x>\nMeeting ID: 555").as_deref(),
            Some("https://teams.microsoft.com/l/meetup-join/abc/0?context=x")
        );
        assert_eq!(
            url("<a href=\"https://example.webex.com/meet/dana?a=1&amp;b=2\">Join</a>").as_deref(),
            Some("https://example.webex.com/meet/dana?a=1&b=2")
        );
        assert_eq!(
            url("Join with Google Meet: https://meet.google.com/abc-defg-hij.").as_deref(),
            Some("https://meet.google.com/abc-defg-hij")
        );
        assert_eq!(
            url("Link:https://meet.google.com/abc-defg-hij, see you there (https://whereby.com/x)")
                .as_deref(),
            Some("https://meet.google.com/abc-defg-hij")
        );
        // A known link inside another link isn't taken, nor one after
        // a host that isn't known.
        assert_eq!(
            url(
                "http://example.com/?to=https://zoom.us/j/1 and https://example.com/https://zoom.us/j/1"
            ),
            None
        );
        assert_eq!(url("Room 4, second floor. Bring the numbers."), None);
        assert_eq!(url(""), None);
        // A look-alike is passed over for the real one after it.
        assert_eq!(
            url("https://zoom.us.example.com/j/1 https://zoom.us/j/2").as_deref(),
            Some("https://zoom.us/j/2")
        );
    }
}
