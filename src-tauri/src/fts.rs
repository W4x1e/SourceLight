//! Turning a search-box string into a safe FTS5 query.
//!
//! FTS5's MATCH grammar has operators (`AND`/`OR`/`NOT`/`NEAR`), quoting, column
//! filters (`col:`), prefix (`*`) and parentheses — so raw user text fed
//! straight in either errors on a stray character or does something surprising
//! (a typed "OR" silently becoming a boolean). This builds a query that treats
//! the input as plain words: each token is quoted (neutralising every operator),
//! the tokens are implicitly ANDed, and the final token gets a `*` so an
//! as-you-type query matches the word the user is still finishing.
//!
//! Pure and dependency-free, so the whole thing is unit-tested; the actual MATCH
//! runs in `db::caption_search`.

/// Splits on anything that isn't alphanumeric, so punctuation and FTS operators
/// can never reach the query grammar. Unicode letters/digits are kept, so
/// accented and non-Latin captions still tokenize.
fn tokenize(raw: &str) -> Vec<String> {
    raw.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .collect()
}

/// Builds an FTS5 MATCH expression from free-text, or `None` if the input holds
/// no searchable token (all whitespace/punctuation) — the caller treats that as
/// "no caption search", not "match nothing".
///
/// Each token becomes a quoted string (so `OR`, `-`, `*`, `"` are literal), the
/// tokens are space-joined (FTS5 implicit AND), and a trailing `*` on the last
/// token makes it a prefix match for incremental typing.
pub fn build_query(raw: &str) -> Option<String> {
    let tokens = tokenize(raw);
    if tokens.is_empty() {
        return None;
    }
    let last = tokens.len() - 1;
    let parts: Vec<String> = tokens
        .iter()
        .enumerate()
        .map(|(i, t)| {
            // A double quote inside a token can't survive tokenize() (it's a
            // split boundary), but double it anyway so the quoting is correct by
            // construction rather than by luck.
            let quoted = format!("\"{}\"", t.replace('"', "\"\""));
            if i == last {
                format!("{quoted}*")
            } else {
                quoted
            }
        })
        .collect();
    Some(parts.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ands_tokens_and_prefixes_the_last() {
        assert_eq!(build_query("red car"), Some("\"red\" \"car\"*".into()));
    }

    #[test]
    fn a_single_word_is_a_prefix_match() {
        assert_eq!(build_query("sunset"), Some("\"sunset\"*".into()));
    }

    #[test]
    fn punctuation_is_stripped_not_passed_through() {
        assert_eq!(
            build_query("sunset, beach!"),
            Some("\"sunset\" \"beach\"*".into())
        );
    }

    #[test]
    fn fts_operators_are_neutralised_to_literals() {
        // "OR" must be a searched word, not a boolean; "*" and quotes can't reach
        // the grammar. Nothing here should be able to produce an FTS syntax error.
        assert_eq!(
            build_query("foo OR bar"),
            Some("\"foo\" \"OR\" \"bar\"*".into()),
        );
    }

    #[test]
    fn blank_or_punctuation_only_is_none() {
        assert_eq!(build_query(""), None);
        assert_eq!(build_query("   "), None);
        assert_eq!(build_query(" ,.-!\"* "), None);
    }

    #[test]
    fn collapses_runs_of_separators() {
        assert_eq!(build_query("a   b"), Some("\"a\" \"b\"*".into()));
    }

    #[test]
    fn keeps_non_latin_word_characters() {
        assert_eq!(build_query("café"), Some("\"café\"*".into()));
    }
}
