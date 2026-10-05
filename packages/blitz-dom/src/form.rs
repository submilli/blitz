use blitz_traits::node_id::NodeId;
use markup5ever::{LocalName, local_name};

use crate::{BaseDocument, ElementData};
use blitz_traits::{
    navigation::NavigationOptions,
    net::{Body, Entry, FormData, Method},
};
use core::str::FromStr;
use std::fmt::Display;

/// https://url.spec.whatwg.org/#default-encode-set
const DEFAULT_ENCODE_SET: percent_encoding::AsciiSet = percent_encoding::CONTROLS
    // Query Set
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'<')
    .add(b'>')
    // Path Set
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}');

impl BaseDocument {
    /// Submits a form with the given form node ID and submitter node ID
    ///
    /// # Arguments
    /// * `node_id` - The ID of the form node to submit
    /// * `submitter_id` - The ID of the node that triggered the submission
    ///
    /// <https://html.spec.whatwg.org/multipage/form-control-infrastructure.html#form-submission-algorithm>
    pub fn submit_form(&self, node_id: NodeId, submitter_id: NodeId) {
        let node = &self.nodes[node_id];
        let Some(element) = node.element_data() else {
            return;
        };

        let entry = match self.form_entry_list(node_id, Some(submitter_id)) {
            Ok(entries) => entries,
            Err(error) => {
                self.navigation_provider.submission_failed(error);
                return;
            }
        };

        let method = get_form_attr(
            self,
            element,
            local_name!("method"),
            submitter_id,
            local_name!("formmethod"),
        )
        .and_then(|method| method.parse::<FormMethod>().ok())
        .unwrap_or(FormMethod::Get);

        let action = get_form_attr(
            self,
            element,
            local_name!("action"),
            submitter_id,
            local_name!("formaction"),
        )
        .unwrap_or_default();

        // "Parse a URL given action... If this fails, return."
        let parsed_action = if action.is_empty() {
            Some(self.document_url(node_id))
        } else {
            self.resolve_url(node_id, action)
        };
        let Some(mut parsed_action) = parsed_action else {
            return;
        };

        let scheme = parsed_action.scheme();

        let enctype = get_form_attr(
            self,
            element,
            local_name!("enctype"),
            submitter_id,
            local_name!("formenctype"),
        )
        .and_then(|enctype| enctype.parse::<RequestContentType>().ok())
        .unwrap_or(RequestContentType::FormUrlEncoded);

        let mut post_resource = Body::Empty;

        match (scheme, method) {
            ("http" | "https" | "data", FormMethod::Get) => {
                let pairs = convert_to_list_of_name_value_pairs(entry);
                let mut query = String::new();
                url::form_urlencoded::Serializer::new(&mut query).extend_pairs(pairs);
                parsed_action.set_query(Some(&query));
            }
            ("http" | "https", FormMethod::Post) => post_resource = Body::Form(entry),
            ("mailto", FormMethod::Get) => {
                let pairs = convert_to_list_of_name_value_pairs(entry);
                parsed_action.query_pairs_mut().extend_pairs(pairs);
            }
            ("mailto", FormMethod::Post) => {
                let pairs = convert_to_list_of_name_value_pairs(entry);
                let body = match enctype {
                    RequestContentType::TextPlain => {
                        let body = encode_text_plain(&pairs);
                        percent_encoding::utf8_percent_encode(&body, &DEFAULT_ENCODE_SET)
                            .to_string()
                    }
                    _ => {
                        let mut body = String::new();
                        url::form_urlencoded::Serializer::new(&mut body).extend_pairs(pairs);
                        body
                    }
                };
                let mut query = if let Some(query) = parsed_action.query() {
                    let mut query = query.to_string();
                    query.push('&');
                    query
                } else {
                    String::new()
                };
                query.push_str("body=");
                query.push_str(&body);
                parsed_action.set_query(Some(&query));
            }
            _ => {
                #[cfg(feature = "tracing")]
                tracing::warn!(
                    "Scheme {} with method {:?} is not implemented",
                    scheme,
                    method
                );
                return;
            }
        }

        let method = method.try_into().unwrap_or_default();

        let navigation_options =
            NavigationOptions::new(parsed_action, Some(enctype.to_string()), self.id())
                .set_document_resource(post_resource)
                .set_method(method);

        self.navigation_provider.navigate_to(navigation_options)
    }
}

fn get_form_attr<'a>(
    doc: &'a BaseDocument,
    form: &'a ElementData,
    form_local: impl PartialEq<LocalName>,
    submitter_id: NodeId,
    submitter_local: impl PartialEq<LocalName>,
) -> Option<&'a str> {
    get_submitter_attr(doc, submitter_id, submitter_local).or_else(|| form.attr(form_local))
}

fn get_submitter_attr(
    doc: &BaseDocument,
    submitter_id: NodeId,
    local_name: impl PartialEq<LocalName>,
) -> Option<&str> {
    doc.get_node(submitter_id)
        .and_then(|node| node.element_data())
        .and_then(|element_data| {
            if element_data.is_submit_button() {
                element_data.attr(local_name)
            } else {
                None
            }
        })
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum FormMethod {
    Get,
    Post,
    Dialog,
}
impl FromStr for FormMethod {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "get" => FormMethod::Get,
            "post" => FormMethod::Post,
            "dialog" => FormMethod::Dialog,
            _ => return Err(()),
        })
    }
}
impl TryFrom<FormMethod> for Method {
    type Error = &'static str;
    fn try_from(method: FormMethod) -> Result<Self, Self::Error> {
        Ok(match method {
            FormMethod::Get => Method::GET,
            FormMethod::Post => Method::POST,
            FormMethod::Dialog => return Err("Dialog is not an HTTP method"),
        })
    }
}
/// Supported content types for HTTP requests
#[derive(Debug, Clone)]
pub enum RequestContentType {
    /// application/x-www-form-urlencoded
    FormUrlEncoded,
    /// multipart/form-data
    MultipartFormData,
    /// text/plain
    TextPlain,
}

impl FromStr for RequestContentType {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "application/x-www-form-urlencoded" => RequestContentType::FormUrlEncoded,
            "multipart/form-data" => RequestContentType::MultipartFormData,
            "text/plain" => RequestContentType::TextPlain,
            _ => return Err(()),
        })
    }
}

impl Display for RequestContentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RequestContentType::FormUrlEncoded => write!(f, "application/x-www-form-urlencoded"),
            RequestContentType::MultipartFormData => write!(f, "multipart/form-data"),
            RequestContentType::TextPlain => write!(f, "text/plain"),
        }
    }
}

/// Converts the entry list to a vector of name-value pairs with normalized line endings
/// https://html.spec.whatwg.org/multipage/form-control-infrastructure.html#converting-an-entry-list-to-a-list-of-name-value-pairs
fn convert_to_list_of_name_value_pairs(form_data: FormData) -> Vec<(String, String)> {
    form_data
        .iter()
        .map(|Entry { name, value }| {
            let name = normalize_line_endings(name.as_ref());
            let value = normalize_line_endings(value.as_ref());
            (name, value)
        })
        .collect()
}

/// Normalizes line endings in a string according to HTML spec
/// Converts single CR or LF to CRLF pairs according to HTML form submission requirements
fn normalize_line_endings(input: &str) -> String {
    // Replace every occurrence of U+000D (CR) not followed by U+000A (LF),
    // and every occurrence of U+000A (LF) not preceded by U+000D (CR),
    // in value, by a string consisting of U+000D (CR) and U+000A (LF).

    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(current) = chars.next() {
        match (current, chars.peek()) {
            ('\r', Some('\n')) => {
                result.push_str("\r\n");
                chars.next();
            }
            ('\r' | '\n', _) => {
                result.push_str("\r\n");
            }
            _ => result.push(current),
        }
    }

    result
}

/// Encodes form data as text/plain according to HTML spec given an slice of name-value pairs
/// https://html.spec.whatwg.org/multipage/form-control-infrastructure.html#text/plain-encoding-algorithm
fn encode_text_plain<T: AsRef<str>, U: AsRef<str>>(input: &[(T, U)]) -> String {
    let mut out = String::new();
    for (name, value) in input {
        out.push_str(name.as_ref());
        out.push('=');
        out.push_str(value.as_ref());
        out.push_str("\r\n");
    }
    out
}
