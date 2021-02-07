use std::cmp::min;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use regex::{escape, Regex, RegexSet};

use crate::path::{Path, PathItem};
use crate::{IntoPattern, Resource, ResourcePath};

const MAX_DYNAMIC_SEGMENTS: usize = 16;

/// ResourceDef describes an entry in resources table
///
/// Resource definition can contain only 16 dynamic segments
#[derive(Clone, Debug)]
pub struct ResourceDef {
    id: u16,
    tp: PatternType,
    name: String,
    pattern: String,
    elements: Vec<PatternElement>,
}

#[derive(Debug, Clone, PartialEq)]
enum PatternElement {
    Str(String),
    Var(String),
}

#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)]
enum PatternType {
    Static(String),
    Prefix(String),
    Dynamic(Regex, Vec<&'static str>, usize),
    DynamicSet(RegexSet, Vec<(Regex, Vec<&'static str>, usize)>),
}

impl ResourceDef {
    /// Parse path pattern and create new `Pattern` instance.
    ///
    /// Panics if path pattern is malformed.
    pub fn new<T: IntoPattern>(path: T) -> Self {
        if path.is_single() {
            let patterns = path.patterns();
            ResourceDef::with_prefix(&patterns[0], false)
        } else {
            let set = path.patterns();
            let mut data = Vec::new();
            let mut re_set = Vec::new();

            for path in set {
                let (pattern, _, _, len) = ResourceDef::parse(&path, false);

                let re = match Regex::new(&pattern) {
                    Ok(re) => re,
                    Err(err) => panic!("Wrong path pattern: \"{}\" {}", path, err),
                };
                // actix creates one router per thread
                let names: Vec<_> = re
                    .capture_names()
                    .filter_map(|name| {
                        name.map(|name| Box::leak(Box::new(name.to_owned())).as_str())
                    })
                    .collect();
                data.push((re, names, len));
                re_set.push(pattern);
            }

            ResourceDef {
                id: 0,
                tp: PatternType::DynamicSet(RegexSet::new(re_set).unwrap(), data),
                elements: Vec::new(),
                name: String::new(),
                pattern: "".to_owned(),
            }
        }
    }

    /// Parse path pattern and create new `Pattern` instance.
    ///
    /// Use `prefix` type instead of `static`.
    ///
    /// Panics if path regex pattern is malformed.
    pub fn prefix(path: &str) -> Self {
        ResourceDef::with_prefix(path, true)
    }

    /// Parse path pattern and create new `Pattern` instance.
    /// Inserts `/` to begging of the pattern.
    ///
    ///
    /// Use `prefix` type instead of `static`.
    ///
    /// Panics if path regex pattern is malformed.
    pub fn root_prefix(path: &str) -> Self {
        ResourceDef::with_prefix(&insert_slash(path), true)
    }

    /// Resource id
    pub fn id(&self) -> u16 {
        self.id
    }

    /// Set resource id
    pub fn set_id(&mut self, id: u16) {
        self.id = id;
    }

    /// Parse path pattern and create new `Pattern` instance with custom prefix
    fn with_prefix(path: &str, for_prefix: bool) -> Self {
        let path = path.to_owned();
        let (pattern, elements, is_dynamic, len) = ResourceDef::parse(&path, for_prefix);

        let tp = if is_dynamic {
            let re = match Regex::new(&pattern) {
                Ok(re) => re,
                Err(err) => panic!("Wrong path pattern: \"{}\" {}", path, err),
            };
            // actix creates one router per thread
            let names = re
                .capture_names()
                .filter_map(|name| {
                    name.map(|name| Box::leak(Box::new(name.to_owned())).as_str())
                })
                .collect();
            PatternType::Dynamic(re, names, len)
        } else if for_prefix {
            PatternType::Prefix(pattern)
        } else {
            PatternType::Static(pattern)
        };

        ResourceDef {
            tp,
            elements,
            id: 0,
            name: String::new(),
            pattern: path,
        }
    }

    /// Resource pattern name
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
        match self.tp {
            PatternType::Static(ref s) => s == path,
            PatternType::Prefix(ref s) => path.starts_with(s),
            PatternType::Dynamic(ref re, _, _) => re.is_match(path),
            PatternType::DynamicSet(ref re, _) => re.is_match(path),
        }
    }

    /// Is prefix path a match against this resource.
    pub fn is_prefix_match(&self, path: &str) -> Option<usize> {
        let p_len = path.len();
        let path = if path.is_empty() { "/" } else { path };

        match self.tp {
            PatternType::Static(ref s) => {
                if s == path {
                    Some(p_len)
                } else {
                    None
                }
            }
            PatternType::Dynamic(ref re, _, len) => {
                if let Some(captures) = re.captures(path) {
                    let mut pos = 0;
                    let mut passed = false;
                    for capture in captures.iter() {
                        if let Some(ref m) = capture {
                            if !passed {
                                passed = true;
                                continue;
                            }

                            pos = m.end();
                        }
                    }
                    Some(pos + len)
                } else {
                    None
                }
            }
            PatternType::Prefix(ref s) => {
                let len = if path == s {
                    s.len()
                } else if path.starts_with(s)
                    && (s.ends_with('/') || path.split_at(s.len()).1.starts_with('/'))
                {
                    if s.ends_with('/') {
                        s.len() - 1
                    } else {
                        s.len()
                    }
                } else {
                    return None;
                };
                Some(min(p_len, len))
            }
            PatternType::DynamicSet(ref re, ref params) => {
                if let Some(idx) = re.matches(path).into_iter().next() {
                    let (ref pattern, _, len) = params[idx];
                    if let Some(captures) = pattern.captures(path) {
                        let mut pos = 0;
                        let mut passed = false;
                        for capture in captures.iter() {
                            if let Some(ref m) = capture {
                                if !passed {
                                    passed = true;
                                    continue;
                                }

                                pos = m.end();
                            }
                        }
                        Some(pos + len)
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
        }
    }

    /// Is the given path and parameters a match against this pattern.
    pub fn match_path<T: ResourcePath>(&self, path: &mut Path<T>) -> bool {
        match self.tp {
            PatternType::Static(ref s) => {
                if s == path.path() {
                    path.skip(path.len() as u16);
                    true
                } else {
                    false
                }
            }
            PatternType::Prefix(ref s) => {
                let r_path = path.path();
                let len = if s == r_path {
                    s.len()
                } else if r_path.starts_with(s)
                    && (s.ends_with('/') || r_path.split_at(s.len()).1.starts_with('/'))
                {
                    if s.ends_with('/') {
                        s.len() - 1
                    } else {
                        s.len()
                    }
                } else {
                    return false;
                };
                let r_path_len = r_path.len();
                path.skip(min(r_path_len, len) as u16);
                true
            }
            PatternType::Dynamic(ref re, ref names, len) => {
                let mut idx = 0;
                let mut pos = 0;
                let mut segments: [PathItem; MAX_DYNAMIC_SEGMENTS] =
                    [PathItem::Static(""); MAX_DYNAMIC_SEGMENTS];

                if let Some(captures) = re.captures(path.path()) {
                    for (no, name) in names.iter().enumerate() {
                        if let Some(m) = captures.name(&name) {
                            idx += 1;
                            pos = m.end();
                            segments[no] = PathItem::Segment(m.start() as u16, m.end() as u16);
                        } else {
                            log::error!(
                                "Dynamic path match but not all segments found: {}",
                                name
                            );
                            return false;
                        }
                    }
                } else {
                    return false;
                }
                for idx in 0..idx {
                    path.add(names[idx], segments[idx]);
                }
                path.skip((pos + len) as u16);
                true
            }
            PatternType::DynamicSet(ref re, ref params) => {
                if let Some(idx) = re.matches(path.path()).into_iter().next() {
                    let (ref pattern, ref names, len) = params[idx];
                    let mut idx = 0;
                    let mut pos = 0;
                    let mut segments: [PathItem; MAX_DYNAMIC_SEGMENTS] =
                        [PathItem::Static(""); MAX_DYNAMIC_SEGMENTS];

                    if let Some(captures) = pattern.captures(path.path()) {
                        for (no, name) in names.iter().enumerate() {
                            if let Some(m) = captures.name(&name) {
                                idx += 1;
                                pos = m.end();
                                segments[no] =
                                    PathItem::Segment(m.start() as u16, m.end() as u16);
                            } else {
                                log::error!(
                                    "Dynamic path match but not all segments found: {}",
                                    name
                                );
                                return false;
                            }
                        }
                    } else {
                        return false;
                    }
                    for idx in 0..idx {
                        path.add(names[idx], segments[idx]);
                    }
                    path.skip((pos + len) as u16);
                    true
                } else {
                    false
                }
            }
        }
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
        match self.tp {
            PatternType::Static(ref s) => {
                if s == res.resource_path().path() && check(res, user_data) {
                    let path = res.resource_path();
                    path.skip(path.len() as u16);
                    true
                } else {
                    false
                }
            }
            PatternType::Prefix(ref s) => {
                let len = {
                    let r_path = res.resource_path().path();
                    if s == r_path {
                        s.len()
                    } else if r_path.starts_with(s)
                        && (s.ends_with('/') || r_path.split_at(s.len()).1.starts_with('/'))
                    {
                        if s.ends_with('/') {
                            s.len() - 1
                        } else {
                            s.len()
                        }
                    } else {
                        return false;
                    }
                };
                if !check(res, user_data) {
                    return false;
                }
                let path = res.resource_path();
                path.skip(min(path.path().len(), len) as u16);
                true
            }
            PatternType::Dynamic(ref re, ref names, len) => {
                let mut idx = 0;
                let mut pos = 0;
                let mut segments: [PathItem; MAX_DYNAMIC_SEGMENTS] =
                    [PathItem::Static(""); MAX_DYNAMIC_SEGMENTS];

                if let Some(captures) = re.captures(res.resource_path().path()) {
                    for (no, name) in names.iter().enumerate() {
                        if let Some(m) = captures.name(&name) {
                            idx += 1;
                            pos = m.end();
                            segments[no] = PathItem::Segment(m.start() as u16, m.end() as u16);
                        } else {
                            log::error!(
                                "Dynamic path match but not all segments found: {}",
                                name
                            );
                            return false;
                        }
                    }
                } else {
                    return false;
                }

                if !check(res, user_data) {
                    return false;
                }

                let path = res.resource_path();
                for idx in 0..idx {
                    path.add(names[idx], segments[idx]);
                }
                path.skip((pos + len) as u16);
                true
            }
            PatternType::DynamicSet(ref re, ref params) => {
                let path = res.resource_path().path();
                if let Some(idx) = re.matches(path).into_iter().next() {
                    let (ref pattern, ref names, len) = params[idx];
                    let mut idx = 0;
                    let mut pos = 0;
                    let mut segments: [PathItem; MAX_DYNAMIC_SEGMENTS] =
                        [PathItem::Static(""); MAX_DYNAMIC_SEGMENTS];

                    if let Some(captures) = pattern.captures(path) {
                        for (no, name) in names.iter().enumerate() {
                            if let Some(m) = captures.name(&name) {
                                idx += 1;
                                pos = m.end();
                                segments[no] =
                                    PathItem::Segment(m.start() as u16, m.end() as u16);
                            } else {
                                log::error!(
                                    "Dynamic path match but not all segments found: {}",
                                    name
                                );
                                return false;
                            }
                        }
                    } else {
                        return false;
                    }

                    if !check(res, user_data) {
                        return false;
                    }

                    let path = res.resource_path();
                    for idx in 0..idx {
                        path.add(names[idx], segments[idx]);
                    }
                    path.skip((pos + len) as u16);
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Build resource path from elements. Returns `true` on success.
    pub fn resource_path<U, I>(&self, path: &mut String, elements: &mut U) -> bool
    where
        U: Iterator<Item = I>,
        I: AsRef<str>,
    {
        match self.tp {
            PatternType::Prefix(ref p) => path.push_str(p),
            PatternType::Static(ref p) => path.push_str(p),
            PatternType::Dynamic(..) => {
                for el in &self.elements {
                    match *el {
                        PatternElement::Str(ref s) => path.push_str(s),
                        PatternElement::Var(_) => {
                            if let Some(val) = elements.next() {
                                path.push_str(val.as_ref())
                            } else {
                                return false;
                            }
                        }
                    }
                }
            }
            PatternType::DynamicSet(..) => {
                return false;
            }
        }
        true
    }

    /// Build resource path from elements. Returns `true` on success.
    pub fn resource_path_named<K, V, S>(
        &self,
        path: &mut String,
        elements: &HashMap<K, V, S>,
    ) -> bool
    where
        K: std::borrow::Borrow<str> + Eq + Hash,
        V: AsRef<str>,
        S: std::hash::BuildHasher,
    {
        match self.tp {
            PatternType::Prefix(ref p) => path.push_str(p),
            PatternType::Static(ref p) => path.push_str(p),
            PatternType::Dynamic(..) => {
                for el in &self.elements {
                    match *el {
                        PatternElement::Str(ref s) => path.push_str(s),
                        PatternElement::Var(ref name) => {
                            if let Some(val) = elements.get(name) {
                                path.push_str(val.as_ref())
                            } else {
                                return false;
                            }
                        }
                    }
                }
            }
            PatternType::DynamicSet(..) => {
                return false;
            }
        }
        true
    }

    fn parse_param(pattern: &str) -> (PatternElement, String, &str, bool) {
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
        let (mut param, mut rem) = pattern.split_at(close_idx + 1);
        param = &param[1..param.len() - 1]; // Remove outer brackets
        let tail = rem == "*";

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
                    rem = &rem[1..];
                    DEFAULT_PATTERN_TAIL
                } else {
                    DEFAULT_PATTERN
                },
            ),
        };
        (
            PatternElement::Var(name.to_string()),
            format!(r"(?P<{}>{})", &name, &pattern),
            rem,
            tail,
        )
    }

    fn parse(
        mut pattern: &str,
        mut for_prefix: bool,
    ) -> (String, Vec<PatternElement>, bool, usize) {
        if pattern.find('{').is_none() {
            // TODO: MSRV: 1.45
            #[allow(clippy::manual_strip)]
            return if pattern.ends_with('*') {
                let path = &pattern[..pattern.len() - 1];
                let re = String::from("^") + path + "(.*)";
                (re, vec![PatternElement::Str(String::from(path))], true, 0)
            } else {
                (
                    String::from(pattern),
                    vec![PatternElement::Str(String::from(pattern))],
                    false,
                    pattern.chars().count(),
                )
            };
        }

        let mut elements = Vec::new();
        let mut re = String::from("^");
        let mut dyn_elements = 0;

        while let Some(idx) = pattern.find('{') {
            let (prefix, rem) = pattern.split_at(idx);
            elements.push(PatternElement::Str(String::from(prefix)));
            re.push_str(&escape(prefix));
            let (param_pattern, re_part, rem, tail) = Self::parse_param(rem);
            if tail {
                for_prefix = true;
            }

            elements.push(param_pattern);
            re.push_str(&re_part);
            pattern = rem;
            dyn_elements += 1;
        }

        elements.push(PatternElement::Str(String::from(pattern)));
        re.push_str(&escape(pattern));

        if dyn_elements > MAX_DYNAMIC_SEGMENTS {
            panic!(
                "Only {} dynamic segments are allowed, provided: {}",
                MAX_DYNAMIC_SEGMENTS, dyn_elements
            );
        }

        if !for_prefix {
            re.push('$');
        }
        (re, elements, true, pattern.chars().count())
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

pub(crate) fn insert_slash(path: &str) -> String {
    let mut path = path.to_owned();
    if !path.is_empty() && !path.starts_with('/') {
        path.insert(0, '/');
    };
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::Uri;
    use std::convert::TryFrom;

    #[test]
    fn test_parse_static() {
        let re = ResourceDef::new("/");
        assert!(re.is_match("/"));
        assert!(!re.is_match("/a"));

        let re = ResourceDef::new("/name");
        assert!(re.is_match("/name"));
        assert!(!re.is_match("/name1"));
        assert!(!re.is_match("/name/"));
        assert!(!re.is_match("/name~"));

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
    }

    #[test]
    fn test_parse_param() {
        let re = ResourceDef::new("/user/{id}");
        assert!(re.is_match("/user/profile"));
        assert!(re.is_match("/user/2345"));
        assert!(!re.is_match("/user/2345/"));
        assert!(!re.is_match("/user/2345/sdg"));

        let mut path = Path::new("/user/profile");
        assert!(re.match_path(&mut path));
        assert_eq!(path.get("id").unwrap(), "profile");

        let mut path = Path::new("/user/1245125");
        assert!(re.match_path(&mut path));
        assert_eq!(path.get("id").unwrap(), "1245125");

        let re = ResourceDef::new("/v{version}/resource/{id}");
        assert!(re.is_match("/v1/resource/320120"));
        assert!(!re.is_match("/v/resource/1"));
        assert!(!re.is_match("/resource"));

        let mut path = Path::new("/v151/resource/adage32");
        assert!(re.match_path(&mut path));
        assert_eq!(path.get("version").unwrap(), "151");
        assert_eq!(path.get("id").unwrap(), "adage32");

        let re = ResourceDef::new("/{id:[[:digit:]]{6}}");
        assert!(re.is_match("/012345"));
        assert!(!re.is_match("/012"));
        assert!(!re.is_match("/01234567"));
        assert!(!re.is_match("/XXXXXX"));

        let mut path = Path::new("/012345");
        assert!(re.match_path(&mut path));
        assert_eq!(path.get("id").unwrap(), "012345");
    }

    #[allow(clippy::cognitive_complexity)]
    #[test]
    fn test_dynamic_set() {
        let re = ResourceDef::new(vec![
            "/user/{id}",
            "/v{version}/resource/{id}",
            "/{id:[[:digit:]]{6}}",
        ]);
        assert!(re.is_match("/user/profile"));
        assert!(re.is_match("/user/2345"));
        assert!(!re.is_match("/user/2345/"));
        assert!(!re.is_match("/user/2345/sdg"));

        let mut path = Path::new("/user/profile");
        assert!(re.match_path(&mut path));
        assert_eq!(path.get("id").unwrap(), "profile");

        let mut path = Path::new("/user/1245125");
        assert!(re.match_path(&mut path));
        assert_eq!(path.get("id").unwrap(), "1245125");

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
    fn test_parse_tail() {
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
    fn test_static_tail() {
        let re = ResourceDef::new("/user*");
        assert!(re.is_match("/user/profile"));
        assert!(re.is_match("/user/2345"));
        assert!(re.is_match("/user/2345/"));
        assert!(re.is_match("/user/2345/sdg"));

        let re = ResourceDef::new("/user/*");
        assert!(re.is_match("/user/profile"));
        assert!(re.is_match("/user/2345"));
        assert!(re.is_match("/user/2345/"));
        assert!(re.is_match("/user/2345/sdg"));
    }

    #[test]
    fn test_parse_urlencoded_param() {
        let re = ResourceDef::new("/user/{id}/test");

        let mut path = Path::new("/user/2345/test");
        assert!(re.match_path(&mut path));
        assert_eq!(path.get("id").unwrap(), "2345");

        let mut path = Path::new("/user/qwe%25/test");
        assert!(re.match_path(&mut path));
        assert_eq!(path.get("id").unwrap(), "qwe%25");

        let uri = Uri::try_from("/user/qwe%25/test").unwrap();
        let mut path = Path::new(uri);
        assert!(re.match_path(&mut path));
        assert_eq!(path.get("id").unwrap(), "qwe%25");
    }

    #[test]
    fn test_resource_prefix() {
        let re = ResourceDef::prefix("/name");
        assert!(re.is_match("/name"));
        assert!(re.is_match("/name/"));
        assert!(re.is_match("/name/test/test"));
        assert!(re.is_match("/name1"));
        assert!(re.is_match("/name~"));

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
    }

    #[test]
    fn test_resource_prefix_dynamic() {
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

        let mut path = Path::new("/test2/subpath1/subpath2/index.html");
        assert!(re.match_path(&mut path));
        assert_eq!(&path["name"], "test2");
        assert_eq!(&path[0], "test2");
    }

    #[test]
    fn test_resource_path() {
        let mut s = String::new();
        let resource = ResourceDef::new("/user/{item1}/test");
        assert!(resource.resource_path(&mut s, &mut (&["user1"]).iter()));
        assert_eq!(s, "/user/user1/test");

        let mut s = String::new();
        let resource = ResourceDef::new("/user/{item1}/{item2}/test");
        assert!(resource.resource_path(&mut s, &mut (&["item", "item2"]).iter()));
        assert_eq!(s, "/user/item/item2/test");

        let mut s = String::new();
        let resource = ResourceDef::new("/user/{item1}/{item2}");
        assert!(resource.resource_path(&mut s, &mut (&["item", "item2"]).iter()));
        assert_eq!(s, "/user/item/item2");

        let mut s = String::new();
        let resource = ResourceDef::new("/user/{item1}/{item2}/");
        assert!(resource.resource_path(&mut s, &mut (&["item", "item2"]).iter()));
        assert_eq!(s, "/user/item/item2/");

        let mut s = String::new();
        assert!(!resource.resource_path(&mut s, &mut (&["item"]).iter()));

        let mut s = String::new();
        assert!(resource.resource_path(&mut s, &mut (&["item", "item2"]).iter()));
        assert_eq!(s, "/user/item/item2/");
        assert!(!resource.resource_path(&mut s, &mut (&["item"]).iter()));

        let mut s = String::new();
        assert!(resource.resource_path(&mut s, &mut vec!["item", "item2"].into_iter()));
        assert_eq!(s, "/user/item/item2/");

        let mut map = HashMap::new();
        map.insert("item1", "item");

        let mut s = String::new();
        assert!(!resource.resource_path_named(&mut s, &map));

        let mut s = String::new();
        map.insert("item2", "item2");
        assert!(resource.resource_path_named(&mut s, &map));
        assert_eq!(s, "/user/item/item2/");
    }
}
