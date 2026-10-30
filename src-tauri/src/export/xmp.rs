//! The XMP packet a JPEG export carries: creator, copyright, where it
//! was taken, and its keywords, caption and people.
//!
//! EXIF's Artist and Copyright tags already hold both lines, and most tools
//! read them. But the tools an agency runs a delivery through — editors'
//! IPTC panels, file browsers, culling tools, the stock-site uploaders — look for them
//! in XMP as `dc:creator` and `dc:rights`, the IPTC Core fields, and show a
//! blank Creator without it. So a JPEG gets both. The same tools read the town
//! and country from XMP only, so the location goes here too — see
//! [`super::location`] — and so do the keywords (`dc:subject`), the caption
//! (`dc:description`) and who is in it (`Iptc4xmpExt:PersonInImage`), which
//! EXIF has no fields for.
//!
//! JPEG only. It is the delivery format; PNG and WebP would each need their own
//! container surgery for a field their consumers rarely read, and they keep the
//! EXIF lines regardless.

use super::exif::ExportMetadata;
use super::location::{PhotoLocation, XMP_NAMESPACES};

/// The packet, or `None` when there is nothing to say.
pub fn packet(m: &ExportMetadata) -> Option<String> {
    let clean = |s: Option<&str>| s.map(str::trim).filter(|s| !s.is_empty()).map(escape);
    let creator = clean(m.creator.as_deref());
    let copyright = clean(m.copyright.as_deref());
    let caption = clean(m.caption.as_deref());
    let title = clean(m.title.as_deref());
    let credit = clean(m.credit.as_deref());
    let usage = clean(m.usage_terms.as_deref());
    let listed = |items: &[String]| -> Vec<String> {
        items
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(escape)
            .collect()
    };
    let keywords = listed(&m.keywords);
    let people = listed(&m.people);
    let located = m
        .location
        .as_ref()
        .map(PhotoLocation::xmp_fields)
        .unwrap_or_default();
    if creator.is_none()
        && copyright.is_none()
        && caption.is_none()
        && title.is_none()
        && credit.is_none()
        && usage.is_none()
        && keywords.is_empty()
        && people.is_empty()
        && located.is_empty()
    {
        return None;
    }

    let mut fields = String::new();
    let mut namespaces = String::new();
    if let Some(t) = title {
        fields.push_str(&format!(
            "<dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">{t}</rdf:li></rdf:Alt></dc:title>"
        ));
    }
    if creator.is_some() {
        // Several creators, "; "-separated as the photo keeps them:
        // split before escaping, whose entities end in ";" too.
        let names: String = m
            .creator
            .as_deref()
            .unwrap_or_default()
            .split(';')
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(|n| format!("<rdf:li>{}</rdf:li>", escape(n)))
            .collect();
        fields.push_str(&format!(
            "<dc:creator><rdf:Seq>{names}</rdf:Seq></dc:creator>"
        ));
    }
    if let Some(r) = copyright {
        fields.push_str(&format!(
            "<dc:rights><rdf:Alt><rdf:li xml:lang=\"x-default\">{r}</rdf:li></rdf:Alt></dc:rights>"
        ));
    }
    if let Some(d) = &caption {
        fields.push_str(&format!(
            "<dc:description><rdf:Alt><rdf:li xml:lang=\"x-default\">{d}</rdf:li></rdf:Alt></dc:description>"
        ));
    }
    // Whose words they are: the model's caption is marked as such.
    let ai_caption = caption.is_some() && m.caption_is_ai;
    let bag = |element: &str, items: &[String]| {
        let items: String = items
            .iter()
            .map(|i| format!("<rdf:li>{i}</rdf:li>"))
            .collect();
        format!("<{element}><rdf:Bag>{items}</rdf:Bag></{element}>")
    };
    if !keywords.is_empty() {
        fields.push_str(&bag("dc:subject", &keywords));
    }
    if let Some(u) = usage {
        namespaces.push_str(" xmlns:xmpRights=\"http://ns.adobe.com/xap/1.0/rights/\"");
        fields.push_str(&format!(
            "<xmpRights:UsageTerms><rdf:Alt><rdf:li xml:lang=\"x-default\">{u}</rdf:li></rdf:Alt></xmpRights:UsageTerms>"
        ));
    }
    if !people.is_empty() {
        namespaces.push_str(" xmlns:Iptc4xmpExt=\"http://iptc.org/std/Iptc4xmpExt/2008-02-29/\"");
        fields.push_str(&bag("Iptc4xmpExt:PersonInImage", &people));
    }
    if !located.is_empty() || credit.is_some() || ai_caption {
        // The location's namespaces include photoshop, which the credit is in.
        for (prefix, uri) in XMP_NAMESPACES {
            namespaces.push_str(&format!(" xmlns:{prefix}=\"{uri}\""));
        }
    }
    if let Some(c) = credit {
        fields.push_str(&format!("<photoshop:Credit>{c}</photoshop:Credit>"));
    }
    if ai_caption {
        fields.push_str(&format!(
            "<photoshop:CaptionWriter>{}</photoshop:CaptionWriter>",
            escape(crate::xmp::AI_CAPTION_WRITER)
        ));
    }
    if !located.is_empty() {
        for (name, value) in &located {
            fields.push_str(&format!("<{name}>{}</{name}>", escape(value)));
        }
    }

    Some(format!(
        "<?xpacket begin=\"\u{FEFF}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\
         <x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\
         <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\
         <rdf:Description rdf:about=\"\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\"{namespaces}>\
         {fields}\
         </rdf:Description></rdf:RDF></x:xmpmeta>\
         <?xpacket end=\"w\"?>"
    ))
}

fn escape(s: &str) -> String {
    crate::xmp::xml_chars(s)
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The namespace an XMP APP1 segment opens with, NUL included.
const XMP_NAMESPACE: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";

/// Inserts `packet` as an APP1 segment after the JPEG's existing application
/// segments — after JFIF and EXIF, where readers expect XMP to sit. Returns the
/// input unchanged if it does not parse as a JPEG or the packet is too large for
/// one segment; metadata is never worth corrupting a file for.
pub fn insert_into_jpeg(jpeg: Vec<u8>, packet: &str) -> Vec<u8> {
    let payload_len = XMP_NAMESPACE.len() + packet.len();
    // The length field counts itself, and is a u16.
    let Ok(segment_len) = u16::try_from(payload_len + 2) else {
        return jpeg;
    };
    if jpeg.len() < 4 || jpeg[0] != 0xFF || jpeg[1] != 0xD8 {
        return jpeg;
    }

    // Walk APPn segments (FFE0..FFEF), each with a big-endian length.
    let mut at = 2;
    while at + 4 <= jpeg.len() && jpeg[at] == 0xFF && (0xE0..=0xEF).contains(&jpeg[at + 1]) {
        let len = u16::from_be_bytes([jpeg[at + 2], jpeg[at + 3]]) as usize;
        if len < 2 || at + 2 + len > jpeg.len() {
            return jpeg;
        }
        at += 2 + len;
    }

    let mut out = Vec::with_capacity(jpeg.len() + 4 + payload_len);
    out.extend_from_slice(&jpeg[..at]);
    out.extend_from_slice(&[0xFF, 0xE1]);
    out.extend_from_slice(&segment_len.to_be_bytes());
    out.extend_from_slice(XMP_NAMESPACE);
    out.extend_from_slice(packet.as_bytes());
    out.extend_from_slice(&jpeg[at..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(creator: Option<&str>, copyright: Option<&str>) -> ExportMetadata {
        ExportMetadata {
            creator: creator.map(String::from),
            copyright: copyright.map(String::from),
            ..Default::default()
        }
    }

    #[test]
    fn nothing_to_say_means_no_packet() {
        assert_eq!(packet(&lines(None, None)), None);
        assert_eq!(packet(&lines(Some("  "), Some(""))), None);
        let blank = ExportMetadata {
            keywords: vec![" ".into()],
            caption: Some("".into()),
            ..Default::default()
        };
        assert_eq!(packet(&blank), None);
    }

    #[test]
    fn both_lines_land_in_their_iptc_core_fields() {
        let p = packet(&lines(Some("Jane Doe"), Some("© 2026 Jane Doe"))).unwrap();
        assert!(p.contains("<dc:creator><rdf:Seq><rdf:li>Jane Doe</rdf:li>"));
        assert!(p.contains("<rdf:li xml:lang=\"x-default\">© 2026 Jane Doe</rdf:li>"));
    }

    #[test]
    fn markup_in_a_name_is_escaped_not_injected() {
        let p = packet(&lines(Some("A & B <studio>"), None)).unwrap();
        assert!(p.contains("A &amp; B &lt;studio&gt;"));
        assert!(!p.contains("<studio>"));
    }

    #[test]
    fn keywords_caption_and_people_ride_along() {
        let m = ExportMetadata {
            keywords: vec!["castle".into(), "landscape".into()],
            caption: Some("A castle on a rock".into()),
            people: vec!["Ada Lovelace".into()],
            ..Default::default()
        };
        let p = packet(&m).unwrap();
        assert!(p.contains("<dc:subject><rdf:Bag><rdf:li>castle</rdf:li><rdf:li>landscape</rdf:li></rdf:Bag></dc:subject>"), "{p}");
        assert!(p.contains("<dc:description><rdf:Alt><rdf:li xml:lang=\"x-default\">A castle on a rock</rdf:li>"), "{p}");
        assert!(
            p.contains("<Iptc4xmpExt:PersonInImage><rdf:Bag><rdf:li>Ada Lovelace</rdf:li>"),
            "{p}"
        );
        assert!(
            p.contains("xmlns:Iptc4xmpExt=\"http://iptc.org/std/Iptc4xmpExt/2008-02-29/\""),
            "{p}"
        );
        // And the sidecar reader — so other editors — reads it all back.
        let back = crate::xmp::parse(&p).unwrap();
        assert_eq!(
            back.keywords,
            vec!["castle".to_string(), "landscape".to_string()]
        );
        assert_eq!(back.description.as_deref(), Some("A castle on a rock"));
        assert_eq!(back.people, vec!["Ada Lovelace".to_string()]);
    }

    #[test]
    fn a_location_alone_is_worth_a_packet() {
        let here = PhotoLocation {
            position: Some((-32.3, 22.5)),
            approximate: true,
            name: Some("Karoo & Co".into()),
            country: Some("South Africa".into()),
            ..Default::default()
        };
        let p = packet(&ExportMetadata {
            location: Some(here),
            ..Default::default()
        })
        .unwrap();
        assert!(p.contains("xmlns:photoshop=\"http://ns.adobe.com/photoshop/1.0/\""));
        assert!(p.contains("<exif:GPSLatitude>32,18.000000S</exif:GPSLatitude>"));
        assert!(p.contains("<exif:GPSLongitude>22,30.000000E</exif:GPSLongitude>"));
        assert!(p.contains("<exif:GPSHPositioningError>5000/1</exif:GPSHPositioningError>"));
        assert!(p.contains("<photoshop:Country>South Africa</photoshop:Country>"));
        assert!(p.contains("<Iptc4xmpCore:Location>Karoo &amp; Co</Iptc4xmpCore:Location>"));
        assert!(!p.contains("photoshop:City"), "no town is guessed");
    }

    #[test]
    fn a_non_jpeg_is_returned_untouched() {
        let png = b"\x89PNG\r\n\x1a\n rest".to_vec();
        assert_eq!(insert_into_jpeg(png.clone(), "x"), png);
    }

    #[test]
    fn a_title_credit_and_terms_go_out_with_the_rights() {
        let p = packet(&ExportMetadata {
            title: Some("Dusk & dawn".into()),
            creator: Some("Ada Lovelace; Bo".into()),
            credit: Some("Northern Light".into()),
            usage_terms: Some("Editorial use only".into()),
            ..Default::default()
        })
        .unwrap();
        assert!(
            p.contains(
                r#"<dc:title><rdf:Alt><rdf:li xml:lang="x-default">Dusk &amp; dawn</rdf:li>"#
            ),
            "{p}"
        );
        assert!(
            p.contains("<rdf:li>Ada Lovelace</rdf:li><rdf:li>Bo</rdf:li>"),
            "{p}"
        );
        assert!(
            p.contains("<photoshop:Credit>Northern Light</photoshop:Credit>"),
            "{p}"
        );
        assert!(
            p.contains("xmlns:photoshop=") && p.contains("xmlns:xmpRights="),
            "{p}"
        );
        let back = crate::xmp::parse(&p).unwrap();
        assert_eq!(back.text.usage_terms.as_deref(), Some("Editorial use only"));
        assert_eq!(back.text.credit.as_deref(), Some("Northern Light"));
    }
}
