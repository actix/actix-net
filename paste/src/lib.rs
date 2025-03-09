//! A minimal implementation of the paste crate, allowing identifier concatenation in macros.

use proc_macro::{
    Delimiter, Group, Ident, Punct, Spacing, Span, TokenStream, TokenTree
};
use std::{iter, str::FromStr};
use std::panic;

#[proc_macro]
pub fn paste(input: TokenStream) -> TokenStream {
    let mut expanded = TokenStream::new();
    let mut tokens = input.into_iter().peekable();

    while let Some(token) = tokens.next() {
        match token {
            TokenTree::Group(group) => {
                let delimiter = group.delimiter();
                let content = group.stream();
                let span = group.span();

                if delimiter == Delimiter::Bracket && is_paste_operation(&content) {
                    // Process [< ... >] paste operation
                    if let Ok(pasted) = process_paste_operation(content, span) {
                        expanded.extend(pasted);
                    } else {
                        // On error, return the original token
                        expanded.extend(iter::once(TokenTree::Group(group)));
                    }
                } else {
                    // Handle nested groups recursively
                    let nested = paste(content);
                    let mut new_group = Group::new(delimiter, nested);
                    new_group.set_span(span);
                    expanded.extend(iter::once(TokenTree::Group(new_group)));
                }
            }
            // Pass through all other tokens unchanged
            _ => expanded.extend(iter::once(token)),
        }
    }

    expanded
}

// Check if a token stream is a paste operation: [< ... >]
fn is_paste_operation(input: &TokenStream) -> bool {
    let mut tokens = input.clone().into_iter();

    match tokens.next() {
        Some(TokenTree::Punct(punct)) if punct.as_char() == '<' => {}
        _ => return false,
    }

    let mut has_content = false;
    for token in tokens {
        match token {
            TokenTree::Punct(punct) if punct.as_char() == '>' => return has_content,
            _ => has_content = true,
        }
    }

    false
}

// Process the content inside [< ... >]
fn process_paste_operation(input: TokenStream, span: Span) -> Result<TokenStream, ()> {
    let mut tokens = input.into_iter();

    // Skip opening '<'
    if let Some(TokenTree::Punct(punct)) = tokens.next() {
        if punct.as_char() != '<' {
            return Err(());
        }
    } else {
        return Err(());
    }

    // Collect and process segments
    let mut segments = Vec::new();
    while let Some(token) = tokens.next() {
        match &token {
            TokenTree::Punct(punct) if punct.as_char() == '>' => break,
            TokenTree::Ident(ident) => segments.push(ident.to_string()),
            TokenTree::Literal(lit) => {
                let lit_str = lit.to_string();
                if lit_str.starts_with('"') && lit_str.ends_with('"') && lit_str.len() >= 2 {
                    segments.push(lit_str[1..lit_str.len() - 1].to_owned());
                } else {
                    segments.push(lit_str);
                }
            },
            TokenTree::Punct(punct) if punct.as_char() == '_' => segments.push("_".to_owned()),
            TokenTree::Punct(punct) if punct.as_char() == ':' => {
                if segments.is_empty() {
                    return Err(());
                }

                // Handle modifiers like :lower, :upper, etc.
                if let Some(TokenTree::Ident(ident)) = tokens.next() {
                    let modifier = ident.to_string();
                    let last = segments.pop().unwrap();

                    let result = match modifier.as_str() {
                        "lower" => last.to_lowercase(),
                        "upper" => last.to_uppercase(),
                        "snake" => to_snake_case(&last),
                        "camel" => to_camel_case(&last),
                        _ => return Err(()),
                    };

                    segments.push(result);
                } else {
                    return Err(());
                }
            },
            _ => return Err(()),
        }
    }

    // Create identifier from the concatenated segments
    let pasted = segments.join("");

    // Convert to a valid Rust identifier
    let ident = match panic::catch_unwind(|| Ident::new(&pasted, span)) {
        Ok(ident) => TokenTree::Ident(ident),
        Err(_) => {
            // If it starts with a number, try to create a literal
            if pasted.starts_with(|c: char| c.is_ascii_digit()) {
                match TokenStream::from_str(&pasted) {
                    Ok(ts) => {
                        if let Some(token) = ts.into_iter().next() {
                            return Ok(iter::once(token).collect());
                        }
                    }
                    Err(_) => {}
                }
            }
            return Err(());
        }
    };

    Ok(iter::once(ident).collect())
}

// Helper function to convert CamelCase to snake_case
fn to_snake_case(input: &str) -> String {
    let mut result = String::new();
    let mut prev = '_';

    for ch in input.chars() {
        if ch.is_uppercase() && prev != '_' {
            result.push('_');
        }
        result.push(ch.to_lowercase().next().unwrap_or(ch));
        prev = ch;
    }

    result
}

// Helper function to convert snake_case to CamelCase
fn to_camel_case(input: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = false;

    for ch in input.chars() {
        if ch == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(ch.to_uppercase().next().unwrap_or(ch));
            capitalize_next = false;
        } else {
            result.push(ch);
        }
    }

    result
}