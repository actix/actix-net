use std::{
    borrow::{Borrow, Cow},
    cmp::min,
    collections::HashMap,
    hash::{BuildHasher, Hash, Hasher},
    mem,
};

use firestorm::{profile_fn, profile_method, profile_section};
use regex::{escape, Regex, RegexSet};

use crate::{
    path::{Path, PathItem},
    IntoPatterns, Patterns, Resource, ResourcePath,
};

const MAX_DYNAMIC_SEGMENTS: usize = 16;

/// Regex flags to allow '.' in regex to match '\n'
///
/// See the docs under: https://docs.rs/regex/1.5.4/regex/#grouping-and-flags
const REGEX_FLAGS: &str = "(?s-m)";

/// Describes an entry in a resource table.
///
/// Resource definition can contain at most 16 dynamic segments.
#[derive(Clone, Debug)]
pub struct ResourceDef {
    id: u16,

    /// Optional name of resource definition. Defaults to "".
    name: String,

    /// Pattern that generated the resource definition.
    // TODO: Sort of, in dynamic set pattern type it is blank, consider change to option.
    pattern: String,

    /// Pattern type.
    pat_type: PatternType,

    /// List of elements that compose the pattern, in order.
    ///
    /// `None` with pattern type is DynamicSet.
    elements: Option<Vec<PatternElement>>,
}

#[derive(Debug, Clone, PartialEq)]
enum PatternElement {
    /// Literal slice of pattern.
    Const(String),

    /// Name of dynamic segment.
    Var(String),

    /// Tail segment. If present in elements list, it will always be last.
    ///
    /// Tail has optional name for patterns like `/foo/{tail}*` vs "/foo/*".
    Tail(Option<String>),
}

#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)]
enum PatternType {
    /// Single constant/literal segment.
    Static(String),

    /// Single constant/literal prefix segment.
    Prefix(String),

    /// Single regular expression and list of dynamic segment names.
    Dynamic(Regex, Vec<&'static str>),

    /// Regular expression set and list of component expressions plus dynamic segment names.
    DynamicSet(RegexSet, Vec<(Regex, Vec<&'static str>)>),
}

impl ResourceDef {
    /// Parse path pattern and create new `Pattern` instance.
    ///
    /// Panics if path pattern is malformed.
    pub fn new<T: IntoPatterns>(path: T) -> Self {
        profile_method!(new);

        match path.patterns() {
            Patterns::Single(pattern) => ResourceDef::from_single_pattern(&pattern, false),

            // since zero length pattern sets are possible
            // just return a useless `ResourceDef`
            Patterns::List(patterns) if patterns.is_empty() => ResourceDef {
                id: 0,
                name: String::new(),
                pattern: String::new(),
                pat_type: PatternType::DynamicSet(RegexSet::empty(), Vec::new()),
                elements: None,
            },

            Patterns::List(patterns) => {
                let mut re_set = Vec::with_capacity(patterns.len());
                let mut pattern_data = Vec::new();

                for pattern in patterns {
                    match ResourceDef::parse(&pattern, false, true) {
                        (PatternType::Dynamic(re, names), _) => {
                            re_set.push(re.as_str().to_owned());
                            pattern_data.push((re, names));
                        }
                        _ => unreachable!(),
                    }
                }

                let pattern_re_set = RegexSet::new(re_set).unwrap();

                ResourceDef {
                    id: 0,
                    name: String::new(),
                    pattern: String::new(),
                    pat_type: PatternType::DynamicSet(pattern_re_set, pattern_data),
                    elements: None,
                }
            }
        }
    }

    /// Parse path pattern and create new `Pattern` instance.
    ///
    /// Use `prefix` type instead of `static`.
    ///
    /// Panics if path regex pattern is malformed.
    pub fn prefix(path: &str) -> Self {
        profile_method!(prefix);
        ResourceDef::from_single_pattern(path, true)
    }

    /// Parse path pattern and create new `Pattern` instance, inserting a `/` to beginning of
    /// the pattern if absent.
    ///
    /// Use `prefix` type instead of `static`.
    ///
    /// Panics if path regex pattern is malformed.
    pub fn root_prefix(path: &str) -> Self {
        profile_method!(root_prefix);
        ResourceDef::from_single_pattern(&insert_slash(path), true)
    }

    /// Resource ID.
    pub fn id(&self) -> u16 {
        self.id
    }

    /// Set resource ID.
    pub fn set_id(&mut self, id: u16) {
        self.id = id;
    }

    /// Parse path pattern and create a new instance
    fn from_single_pattern(pattern: &str, for_prefix: bool) -> Self {
        profile_method!(from_single_pattern);

        let pattern = pattern.to_owned();
        let (pat_type, elements) = ResourceDef::parse(&pattern, for_prefix, false);

        ResourceDef {
            pat_type,
            pattern,
            elements: Some(elements),
            id: 0,
            name: String::new(),
        }
    }

    /// Resource pattern name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Mutable reference to a name of a resource definition.
    pub fn name_mut(&mut self) -> &mut String {
        &mut self.name
    }

    /// Path pattern of the resource
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    /// Check if path matches this pattern.
    #[inline]
    pub fn is_match(&self, path: &str) -> bool {
        profile_method!(is_match);

        match self.pat_type {
            PatternType::Static(ref s) => s == path,
            PatternType::Prefix(ref s) => path.starts_with(s),
            PatternType::Dynamic(ref re, _) => re.is_match(path),
            PatternType::DynamicSet(ref re, _) => re.is_match(path),
        }
    }

    /// Is prefix path a match against this resource.
    pub fn is_prefix_match(&self, path: &str) -> Option<usize> {
        profile_method!(is_prefix_match);

        let path_len = path.len();
        let path = if path.is_empty() { "/" } else { path };

        match self.pat_type {
            PatternType::Static(ref segment) => {
                if segment == path {
                    Some(path_len)
                } else {
                    None
                }
            }

            PatternType::Prefix(ref prefix) => {
                let prefix_len = if path == prefix {
                    // path length === prefix segment length
                    path_len
                } else {
                    let is_slash_next =
                        prefix.ends_with('/') || path.split_at(prefix.len()).1.starts_with('/');

                    if path.starts_with(prefix) && is_slash_next {
                        // enters this branch if segment delimiter ("/") is present after prefix
                        //
                        // i.e., path starts with prefix segment
                        // and prefix segment ends with /
                        // or first character in path after prefix segment length is /
                        //
                        // eg: Prefix("/test/") or Prefix("/test") would match:
                        // - /test/foo
                        // - /test/foo

                        if prefix.ends_with('/') {
                            prefix.len() - 1
                        } else {
                            prefix.len()
                        }
                    } else {
                        return None;
                    }
                };

                Some(min(path_len, prefix_len))
            }

            PatternType::Dynamic(ref re, _) => re.find(path).map(|m| m.end()),

            PatternType::DynamicSet(ref re, ref params) => {
                let idx = re.matches(path).into_iter().next()?;
                let (ref pattern, _) = params[idx];
                pattern.find(path).map(|m| m.end())
            }
        }
    }

    /// Is the given path and parameters a match against this pattern.
    pub fn match_path<T: ResourcePath>(&self, path: &mut Path<T>) -> bool {
        profile_method!(match_path);
        self.match_path_checked(path, &|_, _| true, &Some(()))
    }

    /// Is the given path and parameters a match against this pattern?
    pub fn match_path_checked<R, T, F, U>(
        &self,
        res: &mut R,
        check: &F,
        user_data: &Option<U>,
    ) -> bool
    where
        T: ResourcePath,
        R: Resource<T>,
        F: Fn(&R, &Option<U>) -> bool,
    {
        profile_method!(match_path_checked);

        let mut segments: [PathItem; MAX_DYNAMIC_SEGMENTS] = Default::default();
        let path = res.resource_path();

        let (matched_len, matched_vars) = match self.pat_type {
            PatternType::Static(ref segment) => {
                profile_section!(pattern_static);

                if segment != path.path() {
                    return false;
                }

                (path.path().len(), None)
            }

            PatternType::Prefix(ref prefix) => {
                profile_section!(pattern_dynamic);

                let path_str = path.path();
                let path_len = path_str.len();

                let len = {
                    if prefix == path_str {
                        // prefix length === path length
                        path_len
                    } else {
                        let is_slash_next = prefix.ends_with('/')
                            || path_str.split_at(prefix.len()).1.starts_with('/');

                        if path_str.starts_with(prefix) && is_slash_next {
                            if prefix.ends_with('/') {
                                prefix.len() - 1
                            } else {
                                prefix.len()
                            }
                        } else {
                            return false;
                        }
                    }
                };

                (min(path.path().len(), len), None)
            }

            PatternType::Dynamic(ref re, ref names) => {
                profile_section!(pattern_dynamic);

                let captures = {
                    profile_section!(pattern_dynamic_regex_exec);

                    match re.captures(path.path()) {
                        Some(captures) => captures,
                        _ => return false,
                    }
                };

                {
                    profile_section!(pattern_dynamic_extract_captures);

                    for (no, name) in names.iter().enumerate() {
                        if let Some(m) = captures.name(&name) {
                            segments[no] = PathItem::Segment(m.start() as u16, m.end() as u16);
                        } else {
                            log::error!(
                                "Dynamic path match but not all segments found: {}",
                                name
                            );
                            return false;
                        }
                    }
                };

                (captures[0].len(), Some(names))
            }

            PatternType::DynamicSet(ref re, ref params) => {
                profile_section!(pattern_dynamic_set);

                let path = path.path();
                let (pattern, names) = match re.matches(path).into_iter().next() {
                    Some(idx) => &params[idx],
                    _ => return false,
                };

                let captures = match pattern.captures(path.path()) {
                    Some(captures) => captures,
                    _ => return false,
                };

                for (no, name) in names.iter().enumerate() {
                    if let Some(m) = captures.name(&name) {
                        segments[no] = PathItem::Segment(m.start() as u16, m.end() as u16);
                    } else {
                        log::error!("Dynamic path match but not all segments found: {}", name);
                        return false;
                    }
                }

                (captures[0].len(), Some(names))
            }
        };

        if !check(res, user_data) {
            return false;
        }

        // Modify `path` to skip matched part and store matched segments
        let path = res.resource_path();
        if let Some(vars) = matched_vars {
            for i in 0..vars.len() {
                path.add(vars[i], mem::take(&mut segments[i]));
            }
        }
        path.skip(matched_len as u16);

        true
    }

    /// Builds resource path with a closure that maps variable elements' names to values.
    ///
    /// Unnamed tail pattern elements will receive `None`.
    fn build_resource_path<F, I>(&self, path: &mut String, mut vars: F) -> bool
    where
        F: FnMut(Option<&str>) -> Option<I>,
        I: AsRef<str>,
    {
        for el in match self.elements {
            Some(ref elements) => elements,
            None => return false,
        } {
            match *el {
                PatternElement::Const(ref val) => path.push_str(val),
                PatternElement::Var(ref name) => match vars(Some(name)) {
                    Some(val) => path.push_str(val.as_ref()),
                    _ => return false,
                },
                PatternElement::Tail(ref name) => match vars(name.as_deref()) {
                    Some(val) => path.push_str(val.as_ref()),
                    None => return false,
                },
            }
        }

        true
    }

    /// Builds resource path from elements. Returns `true` on success.
    ///
    /// If resource pattern has a tail segment, an element must be provided for it.
    pub fn resource_path_from_iter<U, I>(&self, path: &mut String, elements: &mut U) -> bool
    where
        U: Iterator<Item = I>,
        I: AsRef<str>,
    {
        profile_method!(resource_path_from_iter);
        self.build_resource_path(path, |_| elements.next())
    }

    // intentionally not deprecated yet
    #[doc(hidden)]
    pub fn resource_path<U, I>(&self, path: &mut String, elements: &mut U) -> bool
    where
        U: Iterator<Item = I>,
        I: AsRef<str>,
    {
        profile_method!(build_resource_path);
        self.resource_path_from_iter(path, elements)
    }

    /// Builds resource path from map of elements. Returns `true` on success.
    ///
    /// If resource pattern has an unnamed tail segment, path building will fail.
    pub fn resource_path_from_map<K, V, S>(
        &self,
        path: &mut String,
        elements: &HashMap<K, V, S>,
    ) -> bool
    where
        K: Borrow<str> + Eq + Hash,
        V: AsRef<str>,
        S: BuildHasher,
    {
        profile_method!(resource_path_from_map);
        self.build_resource_path(path, |name| {
            name.and_then(|name| elements.get(name).map(AsRef::<str>::as_ref))
        })
    }

    // intentionally not deprecated yet
    #[doc(hidden)]
    pub fn resource_path_named<K, V, S>(
        &self,
        path: &mut String,
        elements: &HashMap<K, V, S>,
    ) -> bool
    where
        K: Borrow<str> + Eq + Hash,
        V: AsRef<str>,
        S: BuildHasher,
    {
        self.resource_path_from_map(path, elements)
    }

    /// Build resource path from map of elements, allowing tail segments to be appended.
    ///
    /// If resource pattern does not define a tail segment, the `tail` parameter will be unused.
    /// In this case, use [`resource_path_from_map`][Self::resource_path_from_map] instead.
    ///
    /// Returns `true` on success.
    pub fn resource_path_from_map_with_tail<K, V, S, T>(
        &self,
        path: &mut String,
        elements: &HashMap<K, V, S>,
        tail: T,
    ) -> bool
    where
        K: Borrow<str> + Eq + Hash,
        V: AsRef<str>,
        S: BuildHasher,
        T: AsRef<str>,
    {
        profile_method!(resource_path_from_map_with_tail);
        self.build_resource_path(path, |name| match name {
            Some(name) => elements.get(name).map(AsRef::<str>::as_ref),
            None => Some(tail.as_ref()),
        })
    }

    fn parse_param(pattern: &str) -> (PatternElement, String, &str) {
        profile_method!(parse_param);

        const DEFAULT_PATTERN: &str = "[^/]+";
        const DEFAULT_PATTERN_TAIL: &str = ".*";

        let mut params_nesting = 0usize;
        let close_idx = pattern
            .find(|c| match c {
                '{' => {
                    params_nesting += 1;
                    false
                }
                '}' => {
                    params_nesting -= 1;
                    params_nesting == 0
                }
                _ => false,
            })
            .expect("malformed dynamic segment");

        let (mut param, mut unprocessed) = pattern.split_at(close_idx + 1);

        // remove outer curly brackets
        param = &param[1..param.len() - 1];

        let tail = unprocessed == "*";

        let (name, pattern) = match param.find(':') {
            Some(idx) => {
                if tail {
                    panic!("Custom regex is not supported for remainder match");
                }

                let (name, pattern) = param.split_at(idx);
                (name, &pattern[1..])
            }
            None => (
                param,
                if tail {
                    unprocessed = &unprocessed[1..];
                    DEFAULT_PATTERN_TAIL
                } else {
                    DEFAULT_PATTERN
                },
            ),
        };

        let element = if tail {
            PatternElement::Tail(Some(name.to_string()))
        } else {
            PatternElement::Var(name.to_string())
        };

        let regex = format!(r"(?P<{}>{})", &name, &pattern);

        (element, regex, unprocessed)
    }

    fn parse(
        pattern: &str,
        for_prefix: bool,
        force_dynamic: bool,
    ) -> (PatternType, Vec<PatternElement>) {
        profile_method!(parse);

        let mut unprocessed = pattern;

        if !force_dynamic && unprocessed.find('{').is_none() && !unprocessed.ends_with('*') {
            // pattern is static

            let tp = if for_prefix {
                PatternType::Prefix(unprocessed.to_owned())
            } else {
                PatternType::Static(unprocessed.to_owned())
            };

            return (tp, vec![PatternElement::Const(unprocessed.to_owned())]);
        }

        let mut elements = Vec::new();
        let mut re = format!("{}^", REGEX_FLAGS);
        let mut dyn_elements = 0;
        let mut has_tail_segment = false;

        while let Some(idx) = unprocessed.find('{') {
            let (prefix, rem) = unprocessed.split_at(idx);

            elements.push(PatternElement::Const(prefix.to_owned()));
            re.push_str(&escape(prefix));

            let (param_pattern, re_part, rem) = Self::parse_param(rem);

            if matches!(param_pattern, PatternElement::Tail(_)) {
                has_tail_segment = true;
            }

            elements.push(param_pattern);
            re.push_str(&re_part);

            unprocessed = rem;
            dyn_elements += 1;
        }

        if let Some(path) = unprocessed.strip_suffix('*') {
            // unnamed tail segment

            elements.push(PatternElement::Const(path.to_owned()));
            elements.push(PatternElement::Tail(None));

            re.push_str(&escape(path));
            re.push_str("(.*)");

            dyn_elements += 1;
        } else if !has_tail_segment && !unprocessed.is_empty() {
            // prevent `Const("")` element from being added after last dynamic segment

            elements.push(PatternElement::Const(unprocessed.to_owned()));
            re.push_str(&escape(unprocessed));
        }

        if dyn_elements > MAX_DYNAMIC_SEGMENTS {
            panic!(
                "Only {} dynamic segments are allowed, provided: {}",
                MAX_DYNAMIC_SEGMENTS, dyn_elements
            );
        }

        if !for_prefix && !has_tail_segment {
            re.push('$');
        }

        let re = match Regex::new(&re) {
            Ok(re) => re,
            Err(err) => panic!("Wrong path pattern: \"{}\" {}", pattern, err),
        };

        // `Bok::leak(Box::new(name))` is an intentional memory leak. In typical applications the
        // routing table is only constructed once (per worker) so leak is bounded. If you are
        // constructing `ResourceDef`s more than once in your application's lifecycle you would
        // expect a linear increase in leaked memory over time.
        let names = re
            .capture_names()
            .filter_map(|name| name.map(|name| Box::leak(Box::new(name.to_owned())).as_str()))
            .collect();

        (PatternType::Dynamic(re, names), elements)
    }
}

impl Eq for ResourceDef {}

impl PartialEq for ResourceDef {
    fn eq(&self, other: &ResourceDef) -> bool {
        self.pattern == other.pattern
    }
}

impl Hash for ResourceDef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.pattern.hash(state);
    }
}

impl<'a> From<&'a str> for ResourceDef {
    fn from(path: &'a str) -> ResourceDef {
        ResourceDef::new(path)
    }
}

impl From<String> for ResourceDef {
    fn from(path: String) -> ResourceDef {
        ResourceDef::new(path)
    }
}

pub(crate) fn insert_slash(path: &str) -> Cow<'_, str> {
    profile_fn!(insert_slash);

    if !path.is_empty() && !path.starts_with('/') {
        let mut new_path = String::with_capacity(path.len() + 1);
        new_path.push('/');
        new_path.push_str(path);
        Cow::Owned(new_path)
    } else {
        Cow::Borrowed(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_static() {
        let re = ResourceDef::new("/");
        assert!(re.is_match("/"));
        assert!(!re.is_match("/a"));

        let re = ResourceDef::new("/name");
        assert!(re.is_match("/name"));
        assert!(!re.is_match("/name1"));
        assert!(!re.is_match("/name/"));
        assert!(!re.is_match("/name~"));

        let mut path = Path::new("/name");
        assert!(re.match_path(&mut path));
        assert_eq!(path.unprocessed(), "");

        assert_eq!(re.is_prefix_match("/name"), Some(5));
        assert_eq!(re.is_prefix_match("/name1"), None);
        assert_eq!(re.is_prefix_match("/name/"), None);
        assert_eq!(re.is_prefix_match("/name~"), None);

        let re = ResourceDef::new("/name/");
        assert!(re.is_match("/name/"));
        assert!(!re.is_match("/name"));
        assert!(!re.is_match("/name/gs"));

        let re = ResourceDef::new("/user/profile");
        assert!(re.is_match("/user/profile"));
        assert!(!re.is_match("/user/profile/profile"));

        let mut path = Path::new("/user/profile");
        assert!(re.match_path(&mut path));
        assert_eq!(path.unprocessed(), "");
    }

    #[test]
    fn parse_param() {
        let re = ResourceDef::new("/user/{id}");
        assert!(re.is_match("/user/profile"));
        assert!(re.is_match("/user/2345"));
        assert!(!re.is_match("/user/2345/"));
        assert!(!re.is_match("/user/2345/sdg"));

        let mut path = Path::new("/user/profile");
        assert!(re.match_path(&mut path));
        assert_eq!(path.get("id").unwrap(), "profile");
        assert_eq!(path.unprocessed(), "");

        let mut path = Path::new("/user/1245125");
        assert!(re.match_path(&mut path));
        assert_eq!(path.get("id").unwrap(), "1245125");
        assert_eq!(path.unprocessed(), "");

        let re = ResourceDef::new("/v{version}/resource/{id}");
        assert!(re.is_match("/v1/resource/320120"));
        assert!(!re.is_match("/v/resource/1"));
        assert!(!re.is_match("/resource"));

        let mut path = Path::new("/v151/resource/adage32");
        assert!(re.match_path(&mut path));
        assert_eq!(path.get("version").unwrap(), "151");
        assert_eq!(path.get("id").unwrap(), "adage32");
        assert_eq!(path.unprocessed(), "");

        let re = ResourceDef::new("/{id:[[:digit:]]{6}}");
        assert!(re.is_match("/012345"));
        assert!(!re.is_match("/012"));
        assert!(!re.is_match("/01234567"));
        assert!(!re.is_match("/XXXXXX"));

        let mut path = Path::new("/012345");
        assert!(re.match_path(&mut path));
        assert_eq!(path.get("id").unwrap(), "012345");
        assert_eq!(path.unprocessed(), "");
    }

    #[allow(clippy::cognitive_complexity)]
    #[test]
    fn dynamic_set() {
        let re = ResourceDef::new(vec![
            "/user/{id}",
            "/v{version}/resource/{id}",
            "/{id:[[:digit:]]{6}}",
            "/static",
        ]);
        assert!(re.is_match("/user/profile"));
        assert!(re.is_match("/user/2345"));
        assert!(!re.is_match("/user/2345/"));
        assert!(!re.is_match("/user/2345/sdg"));

        let mut path = Path::new("/user/profile");
        assert!(re.match_path(&mut path));
        assert_eq!(path.get("id").unwrap(), "profile");
        assert_eq!(path.unprocessed(), "");

        let mut path = Path::new("/user/1245125");
        assert!(re.match_path(&mut path));
        assert_eq!(path.get("id").unwrap(), "1245125");
        assert_eq!(path.unprocessed(), "");

        assert!(re.is_match("/v1/resource/320120"));
        assert!(!re.is_match("/v/resource/1"));
        assert!(!re.is_match("/resource"));

        let mut path = Path::new("/v151/resource/adage32");
        assert!(re.match_path(&mut path));
        assert_eq!(path.get("version").unwrap(), "151");
        assert_eq!(path.get("id").unwrap(), "adage32");

        assert!(re.is_match("/012345"));
        assert!(!re.is_match("/012"));
        assert!(!re.is_match("/01234567"));
        assert!(!re.is_match("/XXXXXX"));

        assert!(re.is_match("/static"));
        assert!(!re.is_match("/a/static"));
        assert!(!re.is_match("/static/a"));

        let mut path = Path::new("/012345");
        assert!(re.match_path(&mut path));
        assert_eq!(path.get("id").unwrap(), "012345");

        let re = ResourceDef::new([
            "/user/{id}",
            "/v{version}/resource/{id}",
            "/{id:[[:digit:]]{6}}",
        ]);
        assert!(re.is_match("/user/profile"));
        assert!(re.is_match("/user/2345"));
        assert!(!re.is_match("/user/2345/"));
        assert!(!re.is_match("/user/2345/sdg"));

        let re = ResourceDef::new([
            "/user/{id}".to_string(),
            "/v{version}/resource/{id}".to_string(),
            "/{id:[[:digit:]]{6}}".to_string(),
        ]);
        assert!(re.is_match("/user/profile"));
        assert!(re.is_match("/user/2345"));
        assert!(!re.is_match("/user/2345/"));
        assert!(!re.is_match("/user/2345/sdg"));
    }

    #[test]
    fn parse_tail() {
        let re = ResourceDef::new("/user/-{id}*");

        let mut path = Path::new("/user/-profile");
        assert!(re.match_path(&mut path));
        assert_eq!(path.get("id").unwrap(), "profile");

        let mut path = Path::new("/user/-2345");
        assert!(re.match_path(&mut path));
        assert_eq!(path.get("id").unwrap(), "2345");

        let mut path = Path::new("/user/-2345/");
        assert!(re.match_path(&mut path));
        assert_eq!(path.get("id").unwrap(), "2345/");

        let mut path = Path::new("/user/-2345/sdg");
        assert!(re.match_path(&mut path));
        assert_eq!(path.get("id").unwrap(), "2345/sdg");
    }

    #[test]
    fn static_tail() {
        let re = ResourceDef::new("/user*");
        assert!(re.is_match("/user/profile"));
        assert!(re.is_match("/user/2345"));
        assert!(re.is_match("/user/2345/"));
        assert!(re.is_match("/user/2345/sdg"));
        assert!(!re.is_match("/foo/profile"));

        let re = ResourceDef::new("/user/*");
        assert!(re.is_match("/user/profile"));
        assert!(re.is_match("/user/2345"));
        assert!(re.is_match("/user/2345/"));
        assert!(re.is_match("/user/2345/sdg"));
        assert!(!re.is_match("/foo/profile"));
    }

    #[test]
    fn dynamic_tail() {
        let re = ResourceDef::new("/user/{id}/*");
        assert!(!re.is_match("/user/2345"));
        let mut path = Path::new("/user/2345/sdg");
        assert!(re.match_path(&mut path));
        assert_eq!(path.get("id").unwrap(), "2345");
    }

    #[test]
    fn newline() {
        let re = ResourceDef::new("/user/a\nb");
        assert!(re.is_match("/user/a\nb"));
        assert!(!re.is_match("/user/a\nb/profile"));

        let re = ResourceDef::new("/a{x}b/test/a{y}b");
        let mut path = Path::new("/a\nb/test/a\nb");
        assert!(re.match_path(&mut path));
        assert_eq!(path.get("x").unwrap(), "\n");
        assert_eq!(path.get("y").unwrap(), "\n");

        let re = ResourceDef::new("/user/*");
        assert!(re.is_match("/user/a\nb/"));

        let re = ResourceDef::new("/user/{id}*");
        let mut path = Path::new("/user/a\nb/a\nb");
        assert!(re.match_path(&mut path));
        assert_eq!(path.get("id").unwrap(), "a\nb/a\nb");

        let re = ResourceDef::new("/user/{id:.*}");
        let mut path = Path::new("/user/a\nb/a\nb");
        assert!(re.match_path(&mut path));
        assert_eq!(path.get("id").unwrap(), "a\nb/a\nb");
    }

    #[cfg(feature = "http")]
    #[test]
    fn parse_urlencoded_param() {
        use std::convert::TryFrom;

        let re = ResourceDef::new("/user/{id}/test");

        let mut path = Path::new("/user/2345/test");
        assert!(re.match_path(&mut path));
        assert_eq!(path.get("id").unwrap(), "2345");

        let mut path = Path::new("/user/qwe%25/test");
        assert!(re.match_path(&mut path));
        assert_eq!(path.get("id").unwrap(), "qwe%25");

        let uri = http::Uri::try_from("/user/qwe%25/test").unwrap();
        let mut path = Path::new(uri);
        assert!(re.match_path(&mut path));
        assert_eq!(path.get("id").unwrap(), "qwe%25");
    }

    #[test]
    fn prefix_static() {
        let re = ResourceDef::prefix("/name");

        assert!(re.is_match("/name"));
        assert!(re.is_match("/name/"));
        assert!(re.is_match("/name/test/test"));
        assert!(re.is_match("/name1"));
        assert!(re.is_match("/name~"));

        let mut path = Path::new("/name");
        assert!(re.match_path(&mut path));
        assert_eq!(path.unprocessed(), "");

        let mut path = Path::new("/name/test");
        assert!(re.match_path(&mut path));
        assert_eq!(path.unprocessed(), "/test");

        assert_eq!(re.is_prefix_match("/name"), Some(5));
        assert_eq!(re.is_prefix_match("/name/"), Some(5));
        assert_eq!(re.is_prefix_match("/name/test/test"), Some(5));
        assert_eq!(re.is_prefix_match("/name1"), None);
        assert_eq!(re.is_prefix_match("/name~"), None);

        let re = ResourceDef::prefix("/name/");
        assert!(re.is_match("/name/"));
        assert!(re.is_match("/name/gs"));
        assert!(!re.is_match("/name"));

        let re = ResourceDef::root_prefix("name/");
        assert!(re.is_match("/name/"));
        assert!(re.is_match("/name/gs"));
        assert!(!re.is_match("/name"));

        let mut path = Path::new("/name/gs");
        assert!(re.match_path(&mut path));
        assert_eq!(path.unprocessed(), "/gs");
    }

    #[test]
    fn prefix_dynamic() {
        let re = ResourceDef::prefix("/{name}/");

        assert!(re.is_match("/name/"));
        assert!(re.is_match("/name/gs"));
        assert!(!re.is_match("/name"));

        assert_eq!(re.is_prefix_match("/name/"), Some(6));
        assert_eq!(re.is_prefix_match("/name/gs"), Some(6));
        assert_eq!(re.is_prefix_match("/name"), None);

        let mut path = Path::new("/test2/");
        assert!(re.match_path(&mut path));
        assert_eq!(&path["name"], "test2");
        assert_eq!(&path[0], "test2");
        assert_eq!(path.unprocessed(), "");

        let mut path = Path::new("/test2/subpath1/subpath2/index.html");
        assert!(re.match_path(&mut path));
        assert_eq!(&path["name"], "test2");
        assert_eq!(&path[0], "test2");
        assert_eq!(path.unprocessed(), "subpath1/subpath2/index.html");
    }

    #[test]
    fn build_path_list() {
        let mut s = String::new();
        let resource = ResourceDef::new("/user/{item1}/test");
        assert!(resource.resource_path_from_iter(&mut s, &mut (&["user1"]).iter()));
        assert_eq!(s, "/user/user1/test");

        let mut s = String::new();
        let resource = ResourceDef::new("/user/{item1}/{item2}/test");
        assert!(resource.resource_path_from_iter(&mut s, &mut (&["item", "item2"]).iter()));
        assert_eq!(s, "/user/item/item2/test");

        let mut s = String::new();
        let resource = ResourceDef::new("/user/{item1}/{item2}");
        assert!(resource.resource_path_from_iter(&mut s, &mut (&["item", "item2"]).iter()));
        assert_eq!(s, "/user/item/item2");

        let mut s = String::new();
        let resource = ResourceDef::new("/user/{item1}/{item2}/");
        assert!(resource.resource_path_from_iter(&mut s, &mut (&["item", "item2"]).iter()));
        assert_eq!(s, "/user/item/item2/");

        let mut s = String::new();
        assert!(!resource.resource_path_from_iter(&mut s, &mut (&["item"]).iter()));

        let mut s = String::new();
        assert!(resource.resource_path_from_iter(&mut s, &mut (&["item", "item2"]).iter()));
        assert_eq!(s, "/user/item/item2/");
        assert!(!resource.resource_path_from_iter(&mut s, &mut (&["item"]).iter()));

        let mut s = String::new();
        assert!(
            resource.resource_path_from_iter(&mut s, &mut vec!["item", "item2"].into_iter())
        );
        assert_eq!(s, "/user/item/item2/");
    }

    #[test]
    fn build_path_map() {
        let resource = ResourceDef::new("/user/{item1}/{item2}/");

        let mut map = HashMap::new();
        map.insert("item1", "item");

        let mut s = String::new();
        assert!(!resource.resource_path_from_map(&mut s, &map));

        map.insert("item2", "item2");

        let mut s = String::new();
        assert!(resource.resource_path_from_map(&mut s, &map));
        assert_eq!(s, "/user/item/item2/");
    }

    #[test]
    fn build_path_tail() {
        let resource = ResourceDef::new("/user/{item1}/*");

        let mut s = String::new();
        assert!(!resource.resource_path_from_iter(&mut s, &mut (&["user1"]).iter()));

        let mut s = String::new();
        assert!(resource.resource_path_from_iter(&mut s, &mut (&["user1", "2345"]).iter()));
        assert_eq!(s, "/user/user1/2345");

        let mut s = String::new();
        let mut map = HashMap::new();
        map.insert("item1", "item");
        assert!(!resource.resource_path_from_map(&mut s, &map));

        let mut s = String::new();
        assert!(resource.resource_path_from_map_with_tail(&mut s, &map, "2345"));
        assert_eq!(s, "/user/item/2345");

        let resource = ResourceDef::new("/user/{item1}*");

        let mut s = String::new();
        assert!(!resource.resource_path_from_iter(&mut s, &mut (&[""; 0]).iter()));

        let mut s = String::new();
        assert!(resource.resource_path_from_iter(&mut s, &mut (&["user1"]).iter()));
        assert_eq!(s, "/user/user1");

        let mut s = String::new();
        let mut map = HashMap::new();
        map.insert("item1", "item");
        assert!(resource.resource_path_from_map(&mut s, &map));
        assert_eq!(s, "/user/item");
    }

    #[test]
    fn build_path_tail_when_resource_has_no_tail() {
        let resource = ResourceDef::new("/user/{item1}");

        let mut map = HashMap::new();
        map.insert("item1", "item");
        let mut s = String::new();
        assert!(resource.resource_path_from_map_with_tail(&mut s, &map, "2345"));
        assert_eq!(s, "/user/item");
    }

    #[test]
    #[should_panic]
    fn invalid_dynamic_segment_delimiter() {
        ResourceDef::new("/user/{username");
    }

    #[test]
    #[should_panic]
    fn invalid_dynamic_segment_name() {
        ResourceDef::new("/user/{}");
    }
}
