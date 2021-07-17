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
///
/// # Dynamic Segments
/// TODO
///
/// # Tail Segments
/// TODO
///
/// # Multi-Pattern Resources
/// TODO
#[derive(Clone, Debug)]
pub struct ResourceDef {
    id: u16,

    /// Optional name of resource.
    name: Option<String>,

    /// Pattern that generated the resource definition.
    ///
    /// `None` when pattern type is `DynamicSet`.
    patterns: Patterns,

    /// Pattern type.
    pat_type: PatternType,

    /// List of segments that compose the pattern, in order.
    ///
    /// `None` when pattern type is `DynamicSet`.
    segments: Option<Vec<PatternSegment>>,
}

#[derive(Debug, Clone, PartialEq)]
enum PatternSegment {
    /// Literal slice of pattern.
    Const(String),

    /// Name of dynamic segment.
    Var(String),

    /// Tail segment. If present in segment list, it will always be last.
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
    /// Constructs a new resource definition from patterns.
    ///
    /// Multi-pattern resources can be constructed by providing a slice (or vec) of patterns.
    ///
    /// # Panics
    /// Panics if path pattern is malformed.
    ///
    /// # Examples
    /// ```
    /// use actix_router::ResourceDef;
    ///
    /// let resource = ResourceDef::new("/user/{id}");
    /// assert!(resource.is_match("/user/123"));
    /// assert!(!resource.is_match("/user/123/stars"));
    /// assert!(!resource.is_match("user/1234"));
    /// assert!(!resource.is_match("/foo"));
    ///
    /// let resource = ResourceDef::new(["/profile", "/user/{id}"]);
    /// assert!(resource.is_match("/profile"));
    /// assert!(resource.is_match("/user/123"));
    /// assert!(!resource.is_match("user/123"));
    /// assert!(!resource.is_match("/foo"));
    /// ```
    pub fn new<T: IntoPatterns>(paths: T) -> Self {
        profile_method!(new);

        match paths.patterns() {
            Patterns::Single(pattern) => ResourceDef::from_single_pattern(&pattern, false),

            // since zero length pattern sets are possible
            // just return a useless `ResourceDef`
            Patterns::List(patterns) if patterns.is_empty() => ResourceDef {
                id: 0,
                name: None,
                patterns: Patterns::List(patterns),
                pat_type: PatternType::DynamicSet(RegexSet::empty(), Vec::new()),
                segments: None,
            },

            Patterns::List(patterns) => {
                let mut re_set = Vec::with_capacity(patterns.len());
                let mut pattern_data = Vec::new();

                for pattern in &patterns {
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
                    name: None,
                    patterns: Patterns::List(patterns),
                    pat_type: PatternType::DynamicSet(pattern_re_set, pattern_data),
                    segments: None,
                }
            }
        }
    }

    /// Constructs a new resource definition using a string pattern that performs prefix matching.
    ///
    /// More specifically, the regular expressions generated for matching are different when using
    /// this method vs using `new`; they will not be appended with the `$` meta-character that
    /// matches the end of an input.
    ///
    /// # Panics
    /// Panics if path regex pattern is malformed.
    ///
    /// # Examples
    /// ```
    /// use actix_router::ResourceDef;
    ///
    /// let resource = ResourceDef::prefix("/user/{id}");
    /// assert!(resource.is_match("/user/123"));
    /// assert!(resource.is_match("/user/123/stars"));
    /// assert!(!resource.is_match("user/123"));
    /// assert!(!resource.is_match("user/123/stars"));
    /// assert!(!resource.is_match("/foo"));
    ///
    /// let resource = ResourceDef::prefix("user/{id}");
    /// assert!(resource.is_match("user/123"));
    /// assert!(resource.is_match("user/123/stars"));
    /// assert!(!resource.is_match("/user/123"));
    /// assert!(!resource.is_match("/user/123/stars"));
    /// assert!(!resource.is_match("foo"));
    /// ```
    pub fn prefix(path: &str) -> Self {
        profile_method!(prefix);
        ResourceDef::from_single_pattern(path, true)
    }

    /// Constructs a new resource definition using a string pattern that performs prefix matching,
    /// inserting a `/` to beginning of the pattern if absent.
    ///
    /// # Panics
    /// Panics if path regex pattern is malformed.
    ///
    /// # Examples
    /// ```
    /// use actix_router::ResourceDef;
    ///
    /// let resource = ResourceDef::root_prefix("/user/{id}");
    /// assert!(resource.is_match("/user/123"));
    /// assert!(resource.is_match("/user/123/stars"));
    /// assert!(!resource.is_match("user/123"));
    /// assert!(!resource.is_match("user/123/stars"));
    /// assert!(!resource.is_match("/foo"));
    ///
    /// let resource = ResourceDef::root_prefix("user/{id}");
    /// assert!(resource.is_match("/user/123"));
    /// assert!(resource.is_match("/user/123/stars"));
    /// assert!(!resource.is_match("user/123"));
    /// assert!(!resource.is_match("user/123/stars"));
    /// assert!(!resource.is_match("foo"));
    /// ```
    pub fn root_prefix(path: &str) -> Self {
        profile_method!(root_prefix);
        ResourceDef::from_single_pattern(&insert_slash(path), true)
    }

    /// Returns a numeric resource ID.
    ///
    /// If not explicitly set using [`set_id`][Self::set_id], this will return `0`.
    ///
    /// # Examples
    /// ```
    /// # use actix_router::ResourceDef;
    /// let mut resource = ResourceDef::new("/root");
    /// assert_eq!(resource.id(), 0);
    ///
    /// resource.set_id(42);
    /// assert_eq!(resource.id(), 42);
    /// ```
    pub fn id(&self) -> u16 {
        self.id
    }

    /// Set numeric resource ID.
    ///
    /// # Examples
    /// ```
    /// # use actix_router::ResourceDef;
    /// let mut resource = ResourceDef::new("/root");
    /// resource.set_id(42);
    /// assert_eq!(resource.id(), 42);
    /// ```
    pub fn set_id(&mut self, id: u16) {
        self.id = id;
    }

    /// Returns resource definition name, if set.
    ///
    /// # Examples
    /// ```
    /// # use actix_router::ResourceDef;
    /// let mut resource = ResourceDef::new("/root");
    /// assert!(resource.name().is_none());
    ///
    /// resource.set_name("root");
    /// assert_eq!(resource.name().unwrap(), "root");
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Assigns a new name to the resource.
    ///
    /// # Panics
    /// Panics if `name` is an empty string.
    ///
    /// # Examples
    /// ```
    /// # use actix_router::ResourceDef;
    /// let mut resource = ResourceDef::new("/root");
    /// resource.set_name("root");
    /// assert_eq!(resource.name().unwrap(), "root");
    /// ```
    pub fn set_name(&mut self, name: impl Into<String>) {
        let name = name.into();

        if name.is_empty() {
            panic!("resource name should not be empty");
        }

        self.name = Some(name)
    }

    /// Returns the pattern string that generated the resource definition.
    ///
    /// Returns `None` if definition was constructed with multiple patterns.
    /// See [`patterns_iter`][Self::pattern_iter].
    ///
    /// # Examples
    /// ```
    /// # use actix_router::ResourceDef;
    /// let mut resource = ResourceDef::new("/user/{id}");
    /// assert_eq!(resource.pattern().unwrap(), "/user/{id}");
    ///
    /// let mut resource = ResourceDef::new(["/profile", "/user/{id}"]);
    /// assert!(resource.pattern().is_none());
    pub fn pattern(&self) -> Option<&str> {
        match &self.patterns {
            Patterns::Single(pattern) => Some(pattern.as_str()),
            Patterns::List(_) => None,
        }
    }

    /// Returns iterator of pattern strings that generated the resource definition.
    ///
    /// # Examples
    /// ```
    /// # use actix_router::ResourceDef;
    /// let mut resource = ResourceDef::new("/root");
    /// let mut iter = resource.pattern_iter();
    /// assert_eq!(iter.next().unwrap(), "/root");
    /// assert!(iter.next().is_none());
    ///
    /// let mut resource = ResourceDef::new(["/root", "/backup"]);
    /// let mut iter = resource.pattern_iter();
    /// assert_eq!(iter.next().unwrap(), "/root");
    /// assert_eq!(iter.next().unwrap(), "/backup");
    /// assert!(iter.next().is_none());
    pub fn pattern_iter(&self) -> impl Iterator<Item = &'_ str> {
        struct PatternIter<'a> {
            patterns: &'a Patterns,
            list_idx: usize,
            done: bool,
        }

        impl<'a> Iterator for PatternIter<'a> {
            type Item = &'a str;

            fn next(&mut self) -> Option<Self::Item> {
                match &self.patterns {
                    Patterns::Single(pattern) => {
                        if self.done {
                            return None;
                        }

                        self.done = true;
                        Some(pattern.as_str())
                    }
                    Patterns::List(patterns) if patterns.is_empty() => None,
                    Patterns::List(patterns) => match patterns.get(self.list_idx) {
                        Some(pattern) => {
                            self.list_idx += 1;
                            Some(pattern.as_str())
                        }
                        None => {
                            // fast path future call
                            self.done = true;
                            None
                        }
                    },
                }
            }

            fn size_hint(&self) -> (usize, Option<usize>) {
                match &self.patterns {
                    Patterns::Single(_) => (1, Some(1)),
                    Patterns::List(patterns) => (patterns.len(), Some(patterns.len())),
                }
            }
        }

        PatternIter {
            patterns: &self.patterns,
            list_idx: 0,
            done: false,
        }
    }

    /// Returns `true` if `path` matches this resource.
    ///
    /// The behavior of this method depends on how the `ResourceDef` was constructed. For example,
    /// static resources will not be able to match as many paths as dynamic and prefix resources.
    ///
    /// # Examples
    /// ```
    /// use actix_router::ResourceDef;
    ///
    /// // static resource
    /// let resource = ResourceDef::new("/user");
    /// assert!(resource.is_match("/user"));
    /// assert!(!resource.is_match("/user/123"));
    /// assert!(!resource.is_match("/foo"));
    ///
    /// // dynamic resource
    /// let resource = ResourceDef::new("/user/{user_id}");
    /// assert!(resource.is_match("/user/123"));
    /// assert!(!resource.is_match("/user/123/stars"));
    /// assert!(!resource.is_match("/foo"));
    ///
    /// // prefix resource
    /// let resource = ResourceDef::prefix("/root");
    /// assert!(resource.is_match("/root"));
    /// assert!(resource.is_match("/root/leaf"));
    /// assert!(!resource.is_match("/foo"));
    ///
    /// // TODO: dyn set resource
    /// // TODO: tail segment resource
    /// ```
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

    /// Tries to match prefix of `path` to this resource, returning the position in the path where
    /// the prefix match ends.
    ///
    /// # Examples
    /// ```
    /// use actix_router::ResourceDef;
    ///
    /// // static resource does not do prefix matching
    /// let resource = ResourceDef::new("/user");
    /// assert_eq!(resource.is_prefix_match("/user"), Some(5));
    /// assert!(resource.is_prefix_match("/user/").is_none());
    /// assert!(resource.is_prefix_match("/user/123").is_none());
    /// assert!(resource.is_prefix_match("/foo").is_none());
    ///
    /// // constant prefix resource
    /// let resource = ResourceDef::prefix("/user");
    /// assert_eq!(resource.is_prefix_match("/user"), Some(5));
    /// assert_eq!(resource.is_prefix_match("/user/"), Some(5));
    /// assert_eq!(resource.is_prefix_match("/user/123"), Some(5));
    /// assert!(resource.is_prefix_match("/foo").is_none());
    ///
    /// // dynamic prefix resource
    /// let resource = ResourceDef::prefix("/user/{id}");
    /// assert_eq!(resource.is_prefix_match("/user/123"), Some(9));
    /// assert_eq!(resource.is_prefix_match("/user/123/"), Some(9));
    /// assert_eq!(resource.is_prefix_match("/user/123/stars"), Some(9));
    /// assert!(resource.is_prefix_match("/user/").is_none());
    /// assert!(resource.is_prefix_match("/foo").is_none());
    /// ```
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
                    if path.starts_with(prefix)
                        && (prefix.ends_with('/')
                            || path.split_at(prefix.len()).1.starts_with('/'))
                    {
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

    /// Returns `true` if `path` matches this resource.
    pub fn is_path_match<T: ResourcePath>(&self, path: &mut Path<T>) -> bool {
        profile_method!(is_path_match);
        self.is_path_match_fn(path, &|_, _| true, &None::<()>)
    }

    /// Returns `true` if `path` matches this resource using the supplied check function.
    ///
    /// The check function is supplied with the resource `res` and `user_data`.
    pub fn is_path_match_fn<R, T, F, U>(
        &self,
        res: &mut R,
        check_fn: &F,
        user_data: &Option<U>,
    ) -> bool
    where
        T: ResourcePath,
        R: Resource<T>,
        F: Fn(&R, &Option<U>) -> bool,
    {
        profile_method!(is_path_match_fn);

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
                        // note: see comments in is_prefix_match source

                        if path_str.starts_with(prefix)
                            && (prefix.ends_with('/')
                                || path_str.split_at(prefix.len()).1.starts_with('/'))
                        {
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

        if !check_fn(res, user_data) {
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

    /// Assembles resource path using a closure that maps variable segment names to values.
    ///
    /// Unnamed tail pattern segments will receive `None`.
    fn build_resource_path<F, I>(&self, path: &mut String, mut vars: F) -> bool
    where
        F: FnMut(Option<&str>) -> Option<I>,
        I: AsRef<str>,
    {
        for el in match self.segments {
            Some(ref segments) => segments,
            None => return false,
        } {
            match *el {
                PatternSegment::Const(ref val) => path.push_str(val),
                PatternSegment::Var(ref name) => match vars(Some(name)) {
                    Some(val) => path.push_str(val.as_ref()),
                    _ => return false,
                },
                PatternSegment::Tail(ref name) => match vars(name.as_deref()) {
                    Some(val) => path.push_str(val.as_ref()),
                    None => return false,
                },
            }
        }

        true
    }

    /// Assembles resource path from iterator of dynamic segment values.
    ///
    /// Returns `true` on success.
    ///
    /// If resource pattern has a tail segment, the iterator must be able to provide a value for it.
    pub fn resource_path_from_iter<U, I>(&self, path: &mut String, values: &mut U) -> bool
    where
        U: Iterator<Item = I>,
        I: AsRef<str>,
    {
        profile_method!(resource_path_from_iter);
        self.build_resource_path(path, |_| values.next())
    }

    /// Assembles resource path from map of dynamic segment values.
    ///
    /// Returns `true` on success.
    ///
    /// If resource pattern has an unnamed tail segment, path building will fail.
    /// See [`resource_path_from_map_with_tail`][Self::resource_path_from_map_with_tail] for a
    /// variant of this function that accepts a tail parameter.
    pub fn resource_path_from_map<K, V, S>(
        &self,
        path: &mut String,
        values: &HashMap<K, V, S>,
    ) -> bool
    where
        K: Borrow<str> + Eq + Hash,
        V: AsRef<str>,
        S: BuildHasher,
    {
        profile_method!(resource_path_from_map);
        self.build_resource_path(path, |name| {
            name.and_then(|name| values.get(name).map(AsRef::<str>::as_ref))
        })
    }

    /// Assembles resource path from map of dynamic segment values, allowing tail segments to
    /// be appended.
    ///
    /// Returns `true` on success.
    ///
    /// If resource pattern does not define a tail segment, the `tail` parameter will be unused.
    /// In this case, use [`resource_path_from_map`][Self::resource_path_from_map] instead.
    pub fn resource_path_from_map_with_tail<K, V, S, T>(
        &self,
        path: &mut String,
        values: &HashMap<K, V, S>,
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
            Some(name) => values.get(name).map(AsRef::<str>::as_ref),
            None => Some(tail.as_ref()),
        })
    }

    /// Parse path pattern and create a new instance
    fn from_single_pattern(pattern: &str, is_prefix: bool) -> Self {
        profile_method!(from_single_pattern);

        let pattern = pattern.to_owned();
        let (pat_type, segments) = ResourceDef::parse(&pattern, is_prefix, false);

        ResourceDef {
            id: 0,
            name: None,
            patterns: Patterns::Single(pattern),
            pat_type,
            segments: Some(segments),
        }
    }

    /// Parses a dynamic segment definition from a pattern.
    ///
    /// The returned tuple includes:
    /// - the segment descriptor, either `Var` or `Tail`
    /// - the segment's regex to check values against
    /// - the remaining, unprocessed string slice
    ///
    /// # Panics
    /// Panics if given patterns does not contain a dynamic segment.
    fn parse_param(pattern: &str) -> (PatternSegment, String, &str) {
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

        let segment = if tail {
            PatternSegment::Tail(Some(name.to_string()))
        } else {
            PatternSegment::Var(name.to_string())
        };

        let regex = format!(r"(?P<{}>{})", &name, &pattern);

        (segment, regex, unprocessed)
    }

    /// Parse `pattern` using `is_prefix` and `force_dynamic` flags.
    ///
    /// Parameters:
    /// - `is_prefix`: Use `true` if `pattern` should be treated as a prefix; i.e., a conforming
    ///   path will be a match even if it has parts remaining to process
    /// - `force_dynamic`: Use `true` to disallow the return of static and prefix segments.
    ///
    /// The returned tuple includes:
    /// - the pattern type detected, either `Static`, `Prefix`, or `Dynamic`
    /// - a list of segment descriptors from the pattern
    fn parse(
        pattern: &str,
        is_prefix: bool,
        force_dynamic: bool,
    ) -> (PatternType, Vec<PatternSegment>) {
        profile_method!(parse);

        let mut unprocessed = pattern;

        if !force_dynamic && unprocessed.find('{').is_none() && !unprocessed.ends_with('*') {
            // pattern is static

            let tp = if is_prefix {
                PatternType::Prefix(unprocessed.to_owned())
            } else {
                PatternType::Static(unprocessed.to_owned())
            };

            return (tp, vec![PatternSegment::Const(unprocessed.to_owned())]);
        }

        let mut segments = Vec::new();
        let mut re = format!("{}^", REGEX_FLAGS);
        let mut dyn_segment_count = 0;
        let mut has_tail_segment = false;

        while let Some(idx) = unprocessed.find('{') {
            let (prefix, rem) = unprocessed.split_at(idx);

            segments.push(PatternSegment::Const(prefix.to_owned()));
            re.push_str(&escape(prefix));

            let (param_pattern, re_part, rem) = Self::parse_param(rem);

            if matches!(param_pattern, PatternSegment::Tail(_)) {
                has_tail_segment = true;
            }

            segments.push(param_pattern);
            re.push_str(&re_part);

            unprocessed = rem;
            dyn_segment_count += 1;
        }

        if let Some(path) = unprocessed.strip_suffix('*') {
            // unnamed tail segment

            segments.push(PatternSegment::Const(path.to_owned()));
            segments.push(PatternSegment::Tail(None));

            re.push_str(&escape(path));
            re.push_str("(.*)");

            dyn_segment_count += 1;
        } else if !has_tail_segment && !unprocessed.is_empty() {
            // prevent `Const("")` element from being added after last dynamic segment

            segments.push(PatternSegment::Const(unprocessed.to_owned()));
            re.push_str(&escape(unprocessed));
        }

        if dyn_segment_count > MAX_DYNAMIC_SEGMENTS {
            panic!(
                "Only {} dynamic segments are allowed, provided: {}",
                MAX_DYNAMIC_SEGMENTS, dyn_segment_count
            );
        }

        if !is_prefix && !has_tail_segment {
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

        (PatternType::Dynamic(re, names), segments)
    }
}

impl Eq for ResourceDef {}

impl PartialEq for ResourceDef {
    fn eq(&self, other: &ResourceDef) -> bool {
        self.patterns == other.patterns
    }
}

impl Hash for ResourceDef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.patterns.hash(state);
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
        assert!(re.is_path_match(&mut path));
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
        assert!(re.is_path_match(&mut path));
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
        assert!(re.is_path_match(&mut path));
        assert_eq!(path.get("id").unwrap(), "profile");
        assert_eq!(path.unprocessed(), "");

        let mut path = Path::new("/user/1245125");
        assert!(re.is_path_match(&mut path));
        assert_eq!(path.get("id").unwrap(), "1245125");
        assert_eq!(path.unprocessed(), "");

        let re = ResourceDef::new("/v{version}/resource/{id}");
        assert!(re.is_match("/v1/resource/320120"));
        assert!(!re.is_match("/v/resource/1"));
        assert!(!re.is_match("/resource"));

        let mut path = Path::new("/v151/resource/adage32");
        assert!(re.is_path_match(&mut path));
        assert_eq!(path.get("version").unwrap(), "151");
        assert_eq!(path.get("id").unwrap(), "adage32");
        assert_eq!(path.unprocessed(), "");

        let re = ResourceDef::new("/{id:[[:digit:]]{6}}");
        assert!(re.is_match("/012345"));
        assert!(!re.is_match("/012"));
        assert!(!re.is_match("/01234567"));
        assert!(!re.is_match("/XXXXXX"));

        let mut path = Path::new("/012345");
        assert!(re.is_path_match(&mut path));
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
        assert!(re.is_path_match(&mut path));
        assert_eq!(path.get("id").unwrap(), "profile");
        assert_eq!(path.unprocessed(), "");

        let mut path = Path::new("/user/1245125");
        assert!(re.is_path_match(&mut path));
        assert_eq!(path.get("id").unwrap(), "1245125");
        assert_eq!(path.unprocessed(), "");

        assert!(re.is_match("/v1/resource/320120"));
        assert!(!re.is_match("/v/resource/1"));
        assert!(!re.is_match("/resource"));

        let mut path = Path::new("/v151/resource/adage32");
        assert!(re.is_path_match(&mut path));
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
        assert!(re.is_path_match(&mut path));
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
        assert!(re.is_path_match(&mut path));
        assert_eq!(path.get("id").unwrap(), "profile");

        let mut path = Path::new("/user/-2345");
        assert!(re.is_path_match(&mut path));
        assert_eq!(path.get("id").unwrap(), "2345");

        let mut path = Path::new("/user/-2345/");
        assert!(re.is_path_match(&mut path));
        assert_eq!(path.get("id").unwrap(), "2345/");

        let mut path = Path::new("/user/-2345/sdg");
        assert!(re.is_path_match(&mut path));
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
        assert!(re.is_path_match(&mut path));
        assert_eq!(path.get("id").unwrap(), "2345");
    }

    #[test]
    fn newline() {
        let re = ResourceDef::new("/user/a\nb");
        assert!(re.is_match("/user/a\nb"));
        assert!(!re.is_match("/user/a\nb/profile"));

        let re = ResourceDef::new("/a{x}b/test/a{y}b");
        let mut path = Path::new("/a\nb/test/a\nb");
        assert!(re.is_path_match(&mut path));
        assert_eq!(path.get("x").unwrap(), "\n");
        assert_eq!(path.get("y").unwrap(), "\n");

        let re = ResourceDef::new("/user/*");
        assert!(re.is_match("/user/a\nb/"));

        let re = ResourceDef::new("/user/{id}*");
        let mut path = Path::new("/user/a\nb/a\nb");
        assert!(re.is_path_match(&mut path));
        assert_eq!(path.get("id").unwrap(), "a\nb/a\nb");

        let re = ResourceDef::new("/user/{id:.*}");
        let mut path = Path::new("/user/a\nb/a\nb");
        assert!(re.is_path_match(&mut path));
        assert_eq!(path.get("id").unwrap(), "a\nb/a\nb");
    }

    #[cfg(feature = "http")]
    #[test]
    fn parse_urlencoded_param() {
        use std::convert::TryFrom;

        let re = ResourceDef::new("/user/{id}/test");

        let mut path = Path::new("/user/2345/test");
        assert!(re.is_path_match(&mut path));
        assert_eq!(path.get("id").unwrap(), "2345");

        let mut path = Path::new("/user/qwe%25/test");
        assert!(re.is_path_match(&mut path));
        assert_eq!(path.get("id").unwrap(), "qwe%25");

        let uri = http::Uri::try_from("/user/qwe%25/test").unwrap();
        let mut path = Path::new(uri);
        assert!(re.is_path_match(&mut path));
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
        assert!(re.is_path_match(&mut path));
        assert_eq!(path.unprocessed(), "");

        let mut path = Path::new("/name/test");
        assert!(re.is_path_match(&mut path));
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
        assert!(re.is_path_match(&mut path));
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
        assert!(re.is_path_match(&mut path));
        assert_eq!(&path["name"], "test2");
        assert_eq!(&path[0], "test2");
        assert_eq!(path.unprocessed(), "");

        let mut path = Path::new("/test2/subpath1/subpath2/index.html");
        assert!(re.is_path_match(&mut path));
        assert_eq!(&path["name"], "test2");
        assert_eq!(&path[0], "test2");
        assert_eq!(path.unprocessed(), "subpath1/subpath2/index.html");

        let resource = ResourceDef::prefix("/user");
        // input string shorter than prefix
        assert!(resource.is_prefix_match("/foo").is_none());
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

    #[test]
    #[should_panic]
    fn invalid_too_many_dynamic_segments() {
        // valid
        ResourceDef::new("/{a}/{b}/{c}/{d}/{e}/{f}/{g}/{h}/{i}/{j}/{k}/{l}/{m}/{n}/{o}/{p}");

        // panics
        ResourceDef::new(
            "/{a}/{b}/{c}/{d}/{e}/{f}/{g}/{h}/{i}/{j}/{k}/{l}/{m}/{n}/{o}/{p}/{q}",
        );
    }
}
