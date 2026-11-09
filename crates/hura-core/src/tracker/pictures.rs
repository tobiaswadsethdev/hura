//! Images in a pull request's description and its conversation.
//!
//! A pull request's text is Markdown, which names an image by URL, and the
//! window loads nothing from the network. So an image is fetched here, as a
//! Jira ticket's is, by [`pull_image`] -- and only from the forge's own image
//! hosts: an image a pull request's author pointed at anywhere else stays its
//! alt text, as it would if it were a working-copy file that is not there.
//!
//! **GitHub** keeps a private repository's images private: an upload's URL,
//! `github.com/user-attachments/assets/{uuid}`, answers only to a signed-in
//! browser, and an API token is not one. What GitHub gives the API instead is
//! the text rendered as HTML, whose images point at
//! `private-user-images.githubusercontent.com` with a signature in the query
//! that lasts a few minutes. [`signed`] puts those in place of the URLs the
//! author's Markdown names, matched by the upload's id, which both carry.
//!
//! **Azure DevOps** keeps an upload as an attachment of the pull request, at
//! a `dev.azure.com` URL that takes the same token as everything else.
//!
//! Either forge may have the image as an HTML `<img>` tag rather than as
//! Markdown -- GitHub's editor writes one for every paste -- and the clients
//! draw Markdown with HTML left out, so [`html_images`] makes each tag the
//! Markdown it means.

use super::issue::{Image, image_of};
use super::{Kind, Stored, azure_auth, get_bytes};

/// One image in a pull request's text, by the URL it names there. Refused
/// unless it is on one of the forge's own image hosts; the token goes only to
/// the host it is the token for.
pub fn pull_image(stored: &Stored, url: &str) -> Result<Image, String> {
    let token = super::pull::token_of(stored)?;
    let source = &stored.source;
    let (host, path) = split(url).ok_or_else(|| format!("`{url}` is not an https address"))?;
    let auth = match source.kind {
        Kind::GitHub if github_host(&host, path) => String::new(),
        Kind::AzureDevOps if azure_attachment(&host, path, source.org.as_deref()) => {
            azure_auth(token)
        }
        _ => {
            return Err(format!(
                "{url} is not one of {}'s own images, and only those are read here",
                source.kind.label()
            ));
        }
    };
    image_of(&get_bytes(url, &auth)?)
}

/// `https://host/path`: the host, lowercased, and the path from its slash,
/// or nothing for an address that is not https or has a user in it.
fn split(url: &str) -> Option<(String, &str)> {
    let rest = url.strip_prefix("https://")?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, path) = rest.split_at(end);
    if authority.is_empty() || authority.contains('@') || authority.contains(':') {
        return None;
    }
    Some((authority.to_ascii_lowercase(), path))
}

/// GitHub's own image hosts. None of them is sent the token: a signed URL
/// needs none, and the others are public.
fn github_host(host: &str, path: &str) -> bool {
    match host {
        "private-user-images.githubusercontent.com"
        | "user-images.githubusercontent.com"
        | "camo.githubusercontent.com" => true,
        // A public repository's upload, which redirects to a signed URL.
        "github.com" => path.starts_with("/user-attachments/assets/"),
        _ => false,
    }
}

/// An attachment of a pull request in the tracker's own organisation.
fn azure_attachment(host: &str, path: &str, org: Option<&str>) -> bool {
    let Some(org) = org.filter(|o| !o.is_empty()) else {
        return false;
    };
    let rest = if host == "dev.azure.com" {
        let Some(rest) = path
            .strip_prefix('/')
            .and_then(|p| p.split_once('/'))
            .filter(|(first, _)| first.eq_ignore_ascii_case(org))
            .map(|(_, rest)| rest)
        else {
            return false;
        };
        rest
    } else if host.eq_ignore_ascii_case(&format!("{org}.visualstudio.com")) {
        path.trim_start_matches('/')
    } else {
        return false;
    };
    let path = rest.split(['?', '#']).next().unwrap_or_default();
    path.contains("/_apis/git/repositories/")
        && path.contains("/pullRequests/")
        && path.contains("/attachments/")
        && !path.contains("..")
}

/// Every `<img>` tag outside a code block, as the Markdown image it means.
pub(super) fn html_images(markdown: &str) -> String {
    if !markdown.contains("<img") && !markdown.contains("<IMG") {
        return markdown.to_string();
    }
    let mut out = String::with_capacity(markdown.len());
    let mut fenced = false;
    for line in markdown.split_inclusive('\n') {
        if line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~") {
            fenced = !fenced;
        }
        if fenced {
            out.push_str(line);
        } else {
            out.push_str(&tags_to_markdown(line));
        }
    }
    out
}

fn tags_to_markdown(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(at) = rest.to_ascii_lowercase().find("<img") {
        let Some(end) = rest[at..].find('>') else {
            break;
        };
        let tag = &rest[at..at + end + 1];
        out.push_str(&rest[..at]);
        // A source that would not stay one Markdown link target stays a tag.
        let src = attr(tag, "src")
            .filter(|s| !s.chars().any(|c| c.is_whitespace() || c == '(' || c == ')'));
        match src {
            Some(src) => {
                let alt = attr(tag, "alt").unwrap_or_default();
                let alt: String = alt.chars().filter(|c| !matches!(c, '[' | ']')).collect();
                out.push_str(&format!("![{alt}]({src})"));
            }
            None => out.push_str(tag),
        }
        rest = &rest[at + end + 1..];
    }
    out.push_str(rest);
    out
}

/// An HTML attribute's value, quoted either way, with the entities a URL
/// picks up unescaped.
fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(found) = lower[from..].find(name) {
        let at = from + found;
        from = at + name.len();
        // A whole attribute name: `src`, not the end of `data-src`.
        let before = lower[..at].chars().last();
        if !before.is_some_and(char::is_whitespace) {
            continue;
        }
        let after = lower[from..].trim_start();
        let Some(after) = after.strip_prefix('=') else {
            continue;
        };
        let after = after.trim_start();
        let start = tag.len() - after.len();
        let quote = after.chars().next()?;
        let value = if quote == '"' || quote == '\'' {
            let inner = &tag[start + 1..];
            &inner[..inner.find(quote)?]
        } else {
            let end = after
                .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
                .unwrap_or(after.len());
            &tag[start..start + end]
        };
        let value = value
            .replace("&amp;", "&")
            .replace("&quot;", "\"")
            .replace("&#39;", "'");
        return Some(value);
    }
    None
}

/// The author's Markdown with each GitHub upload it names replaced by the
/// signed URL GitHub's own rendering of it has, where it has one.
pub(super) fn signed(markdown: &str, html: &str) -> String {
    let signed: Vec<String> = srcs(html)
        .into_iter()
        .filter(|s| s.starts_with("https://private-user-images.githubusercontent.com/"))
        .collect();
    if signed.is_empty() {
        return markdown.to_string();
    }
    let mut out = markdown.to_string();
    for url in uploads(markdown) {
        let Some(id) = url.rsplit('/').next().filter(|id| id.len() >= 16) else {
            continue;
        };
        // The signed URL's file is `{n}-{id}.{ext}`.
        if let Some(to) = signed.iter().find(|s| {
            s.split('?')
                .next()
                .and_then(|p| p.rsplit('/').next())
                .is_some_and(|file| file.contains(id))
        }) {
            out = out.replace(&url, to);
        }
    }
    out
}

/// The `src` of every `<img>` in some HTML.
fn srcs(html: &str) -> Vec<String> {
    let lower = html.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(found) = lower[from..].find("<img") {
        let at = from + found;
        let Some(end) = html[at..].find('>') else {
            break;
        };
        if let Some(src) = attr(&html[at..at + end + 1], "src") {
            out.push(src);
        }
        from = at + end + 1;
    }
    out
}

/// GitHub upload URLs named in some Markdown: the current form and the one
/// before it, `github.com/{owner}/{repo}/assets/{user}/{uuid}`.
fn uploads(markdown: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = markdown;
    while let Some(at) = rest.find("https://github.com/") {
        let tail = &rest[at..];
        let end = tail
            .find(|c: char| c.is_whitespace() || matches!(c, ')' | '"' | '\'' | '>' | '<' | ']'))
            .unwrap_or(tail.len());
        let url = &tail[..end];
        let path = &url["https://github.com".len()..];
        let is_upload = path.starts_with("/user-attachments/assets/")
            || path.split('/').nth(3) == Some("assets");
        if is_upload && !out.iter().any(|u| u == url) {
            out.push(url.to_string());
        }
        rest = &tail[end..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_img_tag_becomes_the_markdown_image_it_means() {
        let md = "Before\n<img width=\"500\" alt=\"Login page\" src=\"https://github.com/user-attachments/assets/0f1e2d3c-4b5a-6978-8a9b-0c1d2e3f4a5b\" />\nafter";
        assert_eq!(
            html_images(md),
            "Before\n![Login page](https://github.com/user-attachments/assets/0f1e2d3c-4b5a-6978-8a9b-0c1d2e3f4a5b)\nafter"
        );
        // In a code block it is code.
        let code = "```html\n<img src=\"https://x.example/a.png\">\n```\n";
        assert_eq!(html_images(code), code);
        // `data-src` is not `src`; a tag with no `src` stays as it was.
        assert_eq!(html_images("<img data-src=\"a\">"), "<img data-src=\"a\">");
    }

    #[test]
    fn a_github_upload_is_given_its_signed_address() {
        let id = "0f1e2d3c-4b5a-6978-8a9b-0c1d2e3f4a5b";
        let md = format!(
            "![shot](https://github.com/user-attachments/assets/{id})\n![other](https://example.com/x.png)"
        );
        let html = format!(
            "<p><a href=\"x\"><img src=\"https://private-user-images.githubusercontent.com/1/2-{id}.png?jwt=a.b.c&amp;x=1\" alt=\"shot\"></a></p>"
        );
        assert_eq!(
            signed(&md, &html),
            format!(
                "![shot](https://private-user-images.githubusercontent.com/1/2-{id}.png?jwt=a.b.c&x=1)\n![other](https://example.com/x.png)"
            )
        );
        // No rendering, or none signed: as written.
        assert_eq!(signed(&md, ""), md);
    }

    #[test]
    fn only_the_forges_own_images_are_fetched() {
        let gh = |u: &str| split(u).is_some_and(|(h, p)| github_host(&h, p));
        assert!(gh(
            "https://private-user-images.githubusercontent.com/1/2-x.png?jwt=y"
        ));
        assert!(gh("https://github.com/user-attachments/assets/abc"));
        assert!(!gh("https://github.com/owner/repo/settings"));
        assert!(!gh("https://evil.example/x.png"));
        assert!(!gh("http://user-images.githubusercontent.com/x.png"));
        assert!(!gh(
            "https://user@private-user-images.githubusercontent.com/x"
        ));

        let az = |u: &str| split(u).is_some_and(|(h, p)| azure_attachment(&h, p, Some("contoso")));
        let path = "_apis/git/repositories/r1/pullRequests/7/attachments/image.png";
        assert!(az(&format!("https://dev.azure.com/contoso/p1/{path}")));
        assert!(az(&format!("https://dev.azure.com/Contoso/p1/{path}")));
        assert!(az(&format!("https://contoso.visualstudio.com/p1/{path}")));
        // Another organisation is somebody else's token.
        assert!(!az(&format!("https://dev.azure.com/fabrikam/p1/{path}")));
        assert!(!az(
            "https://dev.azure.com/contoso/p1/_apis/git/repositories/r1/items?path=x"
        ));
        assert!(!az(&format!(
            "https://dev.azure.com.evil.example/contoso/p1/{path}"
        )));
    }
}
