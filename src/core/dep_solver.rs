use std::collections::{HashMap, HashSet, VecDeque};

use semver::{Version, VersionReq};

use crate::core::package::{Package, PackageSource};
use crate::core::registry::RegistryIndex;
use crate::error::error::BallError;

#[derive(Debug, Clone)]
pub struct Dependency {
    pub name: String,
    pub constraint: VersionReq,
    pub optional: bool,
}

#[derive(Debug, Clone)]
pub struct ResolveResult {
    pub packages: Vec<Package>,
    pub unresolved: Vec<String>,
}

pub fn resolve_deps(
    root_name: &str,
    registry: &impl RegistryIndex,
    installed: &HashMap<String, String>,
) -> Result<ResolveResult, BallError> {
    resolve_from(root_name, None, registry, installed)
}

/// Resolve dependencies around an already-fetched root package.
///
/// Used when the root was pinned with `--version` or forced to one `--source`,
/// so the resolver must not re-fetch (and re-resolve) it from the chain.
pub fn resolve_deps_with_root(
    root: &Package,
    registry: &impl RegistryIndex,
    installed: &HashMap<String, String>,
) -> Result<ResolveResult, BallError> {
    resolve_from(&root.name, Some(root), registry, installed)
}

fn resolve_from(
    root_name: &str,
    pinned_root: Option<&Package>,
    registry: &impl RegistryIndex,
    installed: &HashMap<String, String>,
) -> Result<ResolveResult, BallError> {
    let mut resolved: HashMap<String, Package> = HashMap::new();
    let mut pkg_sources: HashMap<String, PackageSource> = HashMap::new();
    let mut graph: HashMap<String, Vec<String>> = HashMap::new();
    let mut system_nodes: HashSet<String> = HashSet::new();
    let mut constraints: HashMap<String, Vec<(String, VersionReq)>> = HashMap::new();
    let mut unresolved: Vec<String> = Vec::new();
    let mut queue: VecDeque<String> = VecDeque::new();

    queue.push_back(root_name.to_string());

    while let Some(name) = queue.pop_front() {
        if resolved.contains_key(&name) {
            // A constraint registered after this package resolved (a deeper
            // path in the graph, or a pinned root) must still be enforced —
            // revalidate against the stored version before skipping (#75).
            if let Some(pkg) = resolved.get(&name) {
                if let Some(parsed_ver) = parse_version_flexible(&pkg.version) {
                    enforce_constraints(&name, &parsed_ver, &constraints)?;
                }
            }
            continue;
        }

        if let Some(root) = pinned_root {
            if name == root.name {
                resolved.insert(name.clone(), root.clone());
                pkg_sources.insert(name.clone(), root.source.clone());
                enqueue_deps(
                    root,
                    &mut queue,
                    &resolved,
                    &mut constraints,
                    &mut graph,
                    &mut system_nodes,
                )?;
                continue;
            }
        }

        if let Some(installed_ver) = installed.get(&name) {
            let pkg = match registry.fetch_package(&name) {
                Ok(p) => p,
                Err(BallError::PackageNotFound(_)) => {
                    // Only tolerated when every depender is a system package
                    // (Debian virtual packages have no manifest in the chain);
                    // anything else is a genuine resolution failure (#73).
                    if !system_virtual_dep(&name, &pkg_sources, &constraints) {
                        unresolved.push(name);
                    }
                    continue;
                }
                Err(e) => return Err(e),
            };
            let parsed_ver = parse_version_flexible(installed_ver).ok_or_else(|| {
                BallError::VersionConflict(format!(
                    "invalid installed version '{}' for '{}'",
                    installed_ver, name
                ))
            })?;

            enforce_constraints(&name, &parsed_ver, &constraints)?;

            resolved.insert(name.clone(), pkg.clone());
            pkg_sources.insert(name.clone(), pkg.source.clone());
            enqueue_deps(
                &pkg,
                &mut queue,
                &resolved,
                &mut constraints,
                &mut graph,
                &mut system_nodes,
            )?;
            continue;
        }

        let pkg = match registry.fetch_package(&name) {
            Ok(p) => p,
            Err(BallError::PackageNotFound(_)) => {
                if !system_virtual_dep(&name, &pkg_sources, &constraints) {
                    unresolved.push(name);
                }
                continue;
            }
            Err(e) => return Err(e),
        };

        let parsed_ver = match parse_version_flexible(&pkg.version) {
            Some(v) => v,
            None => {
                return Err(BallError::PackageManagerError(format!(
                "unparseable version '{}' for '{}' (system package format not supported by semver)",
                pkg.version, name
            )))
            }
        };

        enforce_constraints(&name, &parsed_ver, &constraints)?;

        resolved.insert(name.clone(), pkg.clone());
        pkg_sources.insert(name.clone(), pkg.source.clone());
        enqueue_deps(
            &pkg,
            &mut queue,
            &resolved,
            &mut constraints,
            &mut graph,
            &mut system_nodes,
        )?;
    }

    for name in &unresolved {
        tracing::warn!(
            "dependency '{}' could not be resolved from any configured source",
            name
        );
    }

    // Cycles are a hard error unless every node on the cycle is a system package
    // (Debian/RPM commonly have mutual deps like libc6 <-> libgcc-s1, handled
    // by the native manager) — see #76.
    let all_system_cycle = match find_cycle(&graph) {
        Some(cycle) => {
            if cycle.iter().any(|n| !system_nodes.contains(n)) {
                return Err(BallError::DependencyCycle(format!(
                    "cycle detected: {} -> {}",
                    cycle.join(" -> "),
                    cycle.first().cloned().unwrap_or_default()
                )));
            }
            true
        }
        None => false,
    };

    let order = if all_system_cycle {
        best_effort_order(&graph)
    } else {
        topological_sort(&graph)?
    };

    let mut packages = Vec::new();
    for name in &order {
        if let Some(pkg) = resolved.remove(name) {
            packages.push(pkg);
        }
    }

    // Anything resolved but not covered by the ordering (a tolerated
    // system-sourced cycle cannot be fully sorted) must not silently vanish.
    packages.extend(resolved.into_values());

    Ok(ResolveResult {
        packages,
        unresolved,
    })
}

/// Whether a missing dependency is a tolerated system virtual package: it has
/// no manifest anywhere in the chain, and *every* package that depends on it is
/// itself system-sourced (Debian/RPM roll-up names the native manager handles).
fn system_virtual_dep(
    name: &str,
    pkg_sources: &HashMap<String, PackageSource>,
    constraints: &HashMap<String, Vec<(String, VersionReq)>>,
) -> bool {
    constraints.get(name).is_some_and(|dependers| {
        !dependers.is_empty()
            && dependers
                .iter()
                .all(|(from, _)| pkg_sources.get(from).is_some_and(pkg_is_system))
    })
}

/// Whether a package's source is the native system package manager.
fn pkg_is_system(source: &PackageSource) -> bool {
    matches!(source, PackageSource::System { .. })
}

/// Reject `name`'s version when any registered constraint does not match.
///
/// Shared by every enforcement site — first resolution, installed-version
/// resolution, the already-resolved skip path, and constraint registration.
fn enforce_constraints(
    name: &str,
    version: &Version,
    constraints: &HashMap<String, Vec<(String, VersionReq)>>,
) -> Result<(), BallError> {
    if let Some(dep_cs) = constraints.get(name) {
        for (from_pkg, c) in dep_cs {
            if !c.matches(version) {
                return Err(BallError::VersionConflict(format!(
                    "version {} of '{}' does not satisfy constraint '{}' required by '{}'",
                    version, name, c, from_pkg
                )));
            }
        }
    }
    Ok(())
}

fn enqueue_deps(
    pkg: &Package,
    queue: &mut VecDeque<String>,
    resolved: &HashMap<String, Package>,
    constraints: &mut HashMap<String, Vec<(String, VersionReq)>>,
    graph: &mut HashMap<String, Vec<String>>,
    system_nodes: &mut HashSet<String>,
) -> Result<(), BallError> {
    let deps = parse_dependencies(pkg)?;
    let dep_names: Vec<String> = deps
        .iter()
        .filter(|d| !d.optional)
        .map(|d| d.name.clone())
        .collect();

    graph.insert(pkg.name.clone(), dep_names.clone());

    if pkg_is_system(&pkg.source) {
        system_nodes.insert(pkg.name.clone());
    }

    for dep in &deps {
        {
            let entry = constraints.entry(dep.name.clone()).or_default();
            entry.push((pkg.name.clone(), dep.constraint.clone()));
        }

        if dep.optional {
            continue;
        }

        // A constraint arriving after its target is already resolved must be
        // enforced now — it can never be re-checked by a later pop (#75).
        if let Some(existing) = resolved.get(&dep.name) {
            if let Some(existing_ver) = parse_version_flexible(&existing.version) {
                enforce_constraints(&dep.name, &existing_ver, constraints)?;
            }
        }

        if !resolved.contains_key(&dep.name) && !queue.contains(&dep.name) {
            queue.push_back(dep.name.clone());
        }
    }

    Ok(())
}

fn parse_dependencies(pkg: &Package) -> Result<Vec<Dependency>, BallError> {
    let mut result = Vec::new();
    if let Some(deps) = &pkg.dependencies {
        for dep_str in deps {
            result.push(parse_dependency_line(dep_str)?);
        }
    }
    Ok(result)
}

pub(crate) fn parse_dependency_line(dep_str: &str) -> Result<Dependency, BallError> {
    let trimmed = dep_str.trim();
    let optional = trimmed.starts_with('?');
    let cleaned = if optional {
        trimmed[1..].trim()
    } else {
        trimmed
    };

    let parts: Vec<&str> = cleaned.splitn(2, ' ').collect();
    let name = parts[0].trim().to_string();
    let constraint_str = parts.get(1).map(|s| s.trim()).unwrap_or("*");

    let constraint = match VersionReq::parse(constraint_str) {
        Ok(c) => c,
        Err(e) => {
            return Err(BallError::VersionConflict(format!(
                "malformed version constraint '{}' for '{}': {}",
                constraint_str, name, e
            )));
        }
    };

    Ok(Dependency {
        name,
        constraint,
        optional,
    })
}

/// Parse a version string that may use formats other than strict semver.
///
/// Handles Debian epoch prefixes (`2:1.21-76`), Debian/RPM revision suffixes
/// (`1.21-76`), upstream Fedora release tags (`8.2.2637-20.fc36`), and the
/// zero-padded segments distro versions use freely (`2:8.1.0875-5ubuntu2`),
/// which semver rejects outright.
/// Returns `None` if the cleaned value still cannot be parsed as semver.
pub(crate) fn parse_version_flexible(raw: &str) -> Option<Version> {
    // Try standard parse first for clean semver versions
    if let Ok(v) = Version::parse(raw) {
        if v.pre.is_empty() {
            return Some(v);
        }
    }

    // Strip Debian epoch prefix (e.g., "2:1.21" -> "1.21")
    let stripped = if let Some(colon_pos) = raw.find(':') {
        &raw[colon_pos + 1..]
    } else {
        raw
    };

    // Strip Debian/RPM revision suffix on the first '-' (e.g., "1.21-76" -> "1.21",
    // "8.2.2637-20.fc36" -> "8.2.2637")
    let no_rev = stripped.split('-').next().unwrap_or(stripped);

    // Strip NuGet/build metadata after '+' (e.g., "1.3+build" -> "1.3")
    let no_meta = no_rev.split('+').next().unwrap_or(no_rev);

    // For Debian versions with embedded tags like "1.3.dfsg+really1.3.1",
    // extract only leading numeric segments (e.g., "1.3")
    let clean: String = no_meta
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();

    // Remove trailing dots
    let clean = clean.trim_end_matches('.');

    if clean.is_empty() {
        return None;
    }

    // Normalize to 3 segments (major.minor.patch) for semver compatibility.
    // Leading zeros are dropped first: semver forbids them on a numeric
    // identifier, so `8.1.0875` would otherwise fail to parse at all and the
    // whole version would be reported as unreadable.
    let segments: Vec<String> = clean.split('.').map(strip_leading_zeros).collect();
    let normalized = match segments.len() {
        0 => return None,
        1 => format!("{}.0.0", segments[0]),
        2 => format!("{}.{}.0", segments[0], segments[1]),
        _ => format!("{}.{}.{}", segments[0], segments[1], segments[2]),
    };

    Version::parse(&normalized).ok()
}

/// `0875` -> `875`, `000` -> `0`: the zero padding semver will not accept.
fn strip_leading_zeros(segment: &str) -> String {
    let trimmed = segment.trim_start_matches('0');
    if trimmed.is_empty() {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

fn find_cycle(graph: &HashMap<String, Vec<String>>) -> Option<Vec<String>> {
    #[derive(Clone, Copy, PartialEq)]
    enum Color {
        White,
        Gray,
        Black,
    }

    let mut colors: HashMap<&str, Color> = HashMap::new();
    for name in graph.keys() {
        colors.entry(name).or_insert(Color::White);
    }

    fn visit<'a>(
        node: &'a str,
        graph: &'a HashMap<String, Vec<String>>,
        colors: &mut HashMap<&'a str, Color>,
        path: &mut Vec<&'a str>,
    ) -> Option<Vec<String>> {
        colors.insert(node, Color::Gray);
        path.push(node);

        if let Some(deps) = graph.get(node) {
            for dep in deps {
                match colors.get(dep.as_str()).unwrap_or(&Color::White) {
                    Color::Gray => {
                        let start = path.iter().position(|n| *n == dep.as_str()).unwrap_or(0);
                        let cycle: Vec<&str> = path[start..].to_vec();
                        return Some(cycle.iter().map(|s| s.to_string()).collect());
                    }
                    Color::White => {
                        if let Some(cycle) = visit(dep, graph, colors, path) {
                            return Some(cycle);
                        }
                    }
                    Color::Black => {}
                }
            }
        }

        path.pop();
        colors.insert(node, Color::Black);
        None
    }

    let nodes: Vec<&str> = graph.keys().map(|s| s.as_str()).collect();
    let mut path = Vec::new();
    for name in nodes {
        if colors.get(name) == Some(&Color::White) {
            if let Some(cycle) = visit(name, graph, &mut colors, &mut path) {
                return Some(cycle);
            }
        }
    }

    None
}

#[cfg(test)]
pub(crate) fn detect_cycles(graph: &HashMap<String, Vec<String>>) -> Result<(), BallError> {
    match find_cycle(graph) {
        Some(cycle) => {
            let first = cycle.first().cloned().unwrap_or_default();
            Err(BallError::DependencyCycle(format!(
                "cycle detected: {} -> {}",
                cycle.join(" -> "),
                first
            )))
        }
        None => Ok(()),
    }
}

pub(crate) fn topological_sort(
    graph: &HashMap<String, Vec<String>>,
) -> Result<Vec<String>, BallError> {
    let order = best_effort_order(graph);

    // Defense-in-depth: every edge must place its dependency before its
    // dependent. A violation means the DFS ordered a cyclic graph —
    // `find_cycle` should have caught it first, but a cycle must never reach
    // the install plan silently (#76).
    let index: HashMap<&str, usize> = order
        .iter()
        .enumerate()
        .map(|(i, n)| (n.as_str(), i))
        .collect();
    for (node, deps) in graph {
        for dep in deps {
            if let (Some(&node_i), Some(&dep_i)) =
                (index.get(node.as_str()), index.get(dep.as_str()))
            {
                if dep_i > node_i {
                    return Err(BallError::DependencyCycle(format!(
                        "cycle detected: '{}' must precede '{}'",
                        dep, node
                    )));
                }
            }
        }
    }

    Ok(order)
}

/// Lenient DFS ordering, used only when every cycle on the graph is
/// system-sourced: such a graph cannot be fully sorted, so a workable order
/// beats failure and the native package manager resolves the mutual deps.
fn best_effort_order(graph: &HashMap<String, Vec<String>>) -> Vec<String> {
    let mut visited: HashSet<String> = HashSet::new();
    let mut result: Vec<String> = Vec::new();

    fn dfs(
        node: &str,
        graph: &HashMap<String, Vec<String>>,
        visited: &mut HashSet<String>,
        result: &mut Vec<String>,
    ) {
        if !visited.insert(node.to_string()) {
            return;
        }

        if let Some(deps) = graph.get(node) {
            for dep in deps {
                dfs(dep, graph, visited, result);
            }
        }

        result.push(node.to_string());
    }

    let nodes: Vec<String> = graph.keys().cloned().collect();
    for name in &nodes {
        if !visited.contains(name) {
            dfs(name, graph, &mut visited, &mut result);
        }
    }

    result
}

pub fn get_installed_map(db: &crate::core::db::DbManager) -> HashMap<String, String> {
    let mut map = HashMap::new();
    if let Ok(pkgs) = db.list_packages() {
        for pkg in &pkgs {
            map.insert(pkg.name.clone(), pkg.version.clone());
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::package::PackageSource;

    /// In-memory `RegistryIndex` that serves pre-built packages and reports
    /// `PackageNotFound` for anything else — the sandboxed stand-in for
    /// `RegistryClient` used by the resolver regression tests.
    struct StubIndex {
        packages: HashMap<String, Package>,
    }

    impl RegistryIndex for StubIndex {
        fn fetch_package(&self, name: &str) -> Result<Package, BallError> {
            self.packages
                .get(name)
                .cloned()
                .ok_or_else(|| BallError::PackageNotFound(name.to_string()))
        }
    }

    fn stub_with(packages: Vec<Package>) -> StubIndex {
        StubIndex {
            packages: packages.into_iter().map(|p| (p.name.clone(), p)).collect(),
        }
    }

    fn make_pkg(name: &str, version: &str, deps: Option<Vec<&str>>) -> Package {
        Package {
            name: name.to_string(),
            version: version.to_string(),
            description: None,
            author: None,
            repository: None,
            architectures: None,
            dependencies: deps.map(|d| d.into_iter().map(String::from).collect()),
            sha256: None,
            hash_algorithm: None,
            download_url: None,
            source: PackageSource::GitHub {
                owner: "test".to_string(),
                repo: name.to_string(),
            },
            advisory: None,
            vulnerabilities: Vec::new(),
        }
    }

    fn make_pkg_system(name: &str, version: &str, deps: Option<Vec<&str>>) -> Package {
        let mut pkg = make_pkg(name, version, deps);
        pkg.source = PackageSource::System {
            manager: "apt".to_string(),
        };
        pkg
    }

    #[test]
    fn test_parse_dependency_line_simple() {
        let dep = parse_dependency_line("foo").unwrap();
        assert_eq!(dep.name, "foo");
        assert_eq!(dep.constraint, VersionReq::STAR);
        assert!(!dep.optional);
    }

    #[test]
    fn test_parse_dependency_line_with_constraint() {
        let dep = parse_dependency_line("foo >=1.0").unwrap();
        assert_eq!(dep.name, "foo");
        assert!(!dep.optional);
    }

    #[test]
    fn test_parse_dependency_line_optional() {
        let dep = parse_dependency_line("? foo").unwrap();
        assert_eq!(dep.name, "foo");
        assert!(dep.optional);
        assert_eq!(dep.constraint, VersionReq::STAR);
    }

    #[test]
    fn test_parse_dependency_line_optional_with_constraint() {
        let dep = parse_dependency_line("? bar >=2.0").unwrap();
        assert_eq!(dep.name, "bar");
        assert!(dep.optional);
    }

    #[test]
    fn test_parse_dependency_line_trimmed() {
        let dep = parse_dependency_line("  baz  ").unwrap();
        assert_eq!(dep.name, "baz");
    }

    #[test]
    fn test_parse_dependency_line_rejects_malformed_constraint() {
        let result = parse_dependency_line("foo >=1.2..3");
        match result {
            Err(BallError::VersionConflict(msg)) => {
                assert!(msg.contains(">=1.2..3"));
                assert!(msg.contains("foo"));
            }
            other => panic!("expected VersionConflict, got {:?}", other),
        }

        assert!(parse_dependency_line("? bar ^'").is_err());
        assert!(parse_dependency_line("baz 1..0").is_err());
    }

    #[test]
    fn test_parse_dependency_line_empty_constraint_stays_star() {
        let dep = parse_dependency_line("foo").unwrap();
        assert_eq!(dep.constraint, VersionReq::STAR);
    }

    #[test]
    fn test_detect_cycles_no_cycle() {
        let mut graph = HashMap::new();
        graph.insert("a".to_string(), vec!["b".to_string()]);
        graph.insert("b".to_string(), vec!["c".to_string()]);
        graph.insert("c".to_string(), vec![]);
        assert!(detect_cycles(&graph).is_ok());
    }

    #[test]
    fn test_detect_cycles_direct() {
        let mut graph = HashMap::new();
        graph.insert("a".to_string(), vec!["a".to_string()]);
        let result = detect_cycles(&graph);
        assert!(result.is_err());
        match result.unwrap_err() {
            BallError::DependencyCycle(msg) => assert!(msg.contains("cycle")),
            _ => panic!("expected DependencyCycle"),
        }
    }

    #[test]
    fn test_detect_cycles_indirect() {
        let mut graph = HashMap::new();
        graph.insert("a".to_string(), vec!["b".to_string()]);
        graph.insert("b".to_string(), vec!["c".to_string()]);
        graph.insert("c".to_string(), vec!["a".to_string()]);
        let result = detect_cycles(&graph);
        assert!(result.is_err());
    }

    #[test]
    fn test_detect_cycles_disjoint() {
        let mut graph = HashMap::new();
        graph.insert("a".to_string(), vec!["b".to_string()]);
        graph.insert("b".to_string(), vec![]);
        graph.insert("c".to_string(), vec!["d".to_string()]);
        graph.insert("d".to_string(), vec![]);
        assert!(detect_cycles(&graph).is_ok());
    }

    #[test]
    fn test_detect_cycles_empty() {
        let graph = HashMap::new();
        assert!(detect_cycles(&graph).is_ok());
    }

    #[test]
    fn test_topological_sort_simple() {
        let mut graph = HashMap::new();
        graph.insert("a".to_string(), vec!["b".to_string()]);
        graph.insert("b".to_string(), vec![]);
        let order = topological_sort(&graph).unwrap();
        assert_eq!(order, vec!["b", "a"]);
    }

    #[test]
    fn test_topological_sort_multi_level() {
        let mut graph = HashMap::new();
        graph.insert("a".to_string(), vec!["b".to_string(), "c".to_string()]);
        graph.insert("b".to_string(), vec!["d".to_string()]);
        graph.insert("c".to_string(), vec!["d".to_string()]);
        graph.insert("d".to_string(), vec![]);
        let order = topological_sort(&graph).unwrap();
        assert_eq!(order.len(), 4);
        assert!(order.iter().position(|n| n == "d") < order.iter().position(|n| n == "b"));
        assert!(order.iter().position(|n| n == "d") < order.iter().position(|n| n == "c"));
        assert!(order.iter().position(|n| n == "b") < order.iter().position(|n| n == "a"));
        assert!(order.iter().position(|n| n == "c") < order.iter().position(|n| n == "a"));
    }

    #[test]
    fn test_topological_sort_empty() {
        let graph = HashMap::new();
        let order = topological_sort(&graph).unwrap();
        assert!(order.is_empty());
    }

    #[test]
    fn test_topological_sort_single() {
        let mut graph = HashMap::new();
        graph.insert("a".to_string(), vec![]);
        let order = topological_sort(&graph).unwrap();
        assert_eq!(order, vec!["a"]);
    }

    #[test]
    fn test_topological_sort_disjoint() {
        let mut graph = HashMap::new();
        graph.insert("a".to_string(), vec![]);
        graph.insert("b".to_string(), vec![]);
        let order = topological_sort(&graph).unwrap();
        assert_eq!(order.len(), 2);
    }

    #[test]
    fn test_parse_dependencies_none() {
        let pkg = make_pkg("test", "1.0", None);
        let deps = parse_dependencies(&pkg).unwrap();
        assert!(deps.is_empty());
    }

    #[test]
    fn test_parse_dependencies_empty_vec() {
        let pkg = make_pkg("test", "1.0", Some(vec![]));
        let deps = parse_dependencies(&pkg).unwrap();
        assert!(deps.is_empty());
    }

    #[test]
    fn test_parse_dependencies_multiple() {
        let pkg = make_pkg("test", "1.0", Some(vec!["dep1", "? dep2", "dep3 >=2.0"]));
        let deps = parse_dependencies(&pkg).unwrap();
        assert_eq!(deps.len(), 3);
        assert_eq!(deps[0].name, "dep1");
        assert!(!deps[0].optional);
        assert_eq!(deps[1].name, "dep2");
        assert!(deps[1].optional);
        assert_eq!(deps[2].name, "dep3");
        assert!(!deps[2].optional);
    }

    #[test]
    fn test_parse_dependencies_propagates_malformed_constraint() {
        let pkg = make_pkg("test", "1.0", Some(vec!["ok", "bad >=1.2..3"]));
        assert!(matches!(
            parse_dependencies(&pkg),
            Err(BallError::VersionConflict(_))
        ));
    }

    #[test]
    fn test_late_constraint_on_installed_dep_is_enforced() {
        let root_a = make_pkg("a", "1.0.0", Some(vec!["b", "c"]));
        let pkg_b = make_pkg("b", "1.0.0", None);
        let pkg_c = make_pkg("c", "1.0.0", Some(vec!["b >=2.0"]));
        let stub = stub_with(vec![pkg_b, pkg_c]);

        let mut installed = HashMap::new();
        installed.insert("b".to_string(), "1.0.0".to_string());

        let result = resolve_deps_with_root(&root_a, &stub, &installed);
        assert!(matches!(result, Err(BallError::VersionConflict(_))));
    }

    #[test]
    fn test_late_constraint_on_fetched_dep_is_enforced() {
        let root_a = make_pkg("a", "1.0.0", Some(vec!["b", "c"]));
        let pkg_b = make_pkg("b", "1.0.0", None);
        let pkg_c = make_pkg("c", "1.0.0", Some(vec!["b >=2.0"]));
        let stub = stub_with(vec![pkg_b, pkg_c]);
        let installed = HashMap::new();

        let result = resolve_deps_with_root(&root_a, &stub, &installed);
        assert!(matches!(result, Err(BallError::VersionConflict(_))));
    }

    #[test]
    fn test_late_constraint_on_pinned_root_is_enforced() {
        let root_a = make_pkg("a", "9.9.9", Some(vec!["b"]));
        let pkg_b = make_pkg("b", "1.0.0", Some(vec!["a <1.0"]));
        let stub = stub_with(vec![pkg_b]);
        let installed = HashMap::new();

        let result = resolve_deps_with_root(&root_a, &stub, &installed);
        assert!(matches!(result, Err(BallError::VersionConflict(_))));
    }

    #[test]
    fn test_compatible_late_constraint_resolves() {
        let root_a = make_pkg("a", "1.0.0", Some(vec!["b", "c"]));
        let pkg_b = make_pkg("b", "2.0.0", None);
        let pkg_c = make_pkg("c", "1.0.0", Some(vec!["b >=2.0"]));
        let stub = stub_with(vec![pkg_b, pkg_c]);
        let installed = HashMap::new();

        let result = resolve_deps_with_root(&root_a, &stub, &installed).unwrap();
        let names: Vec<&str> = result.packages.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"b"));
    }

    #[test]
    fn test_resolver_errors_on_non_system_cycle() {
        let root_a = make_pkg("a", "1.0.0", Some(vec!["b"]));
        let pkg_b = make_pkg("b", "1.0.0", Some(vec!["c"]));
        let pkg_c = make_pkg("c", "1.0.0", Some(vec!["a"]));
        let stub = stub_with(vec![pkg_b, pkg_c]);
        let installed = HashMap::new();

        let result = resolve_deps_with_root(&root_a, &stub, &installed);
        assert!(matches!(result, Err(BallError::DependencyCycle(_))));
    }

    #[test]
    fn test_resolver_errors_on_mixed_cycle() {
        let root_a = make_pkg("a", "1.0.0", Some(vec!["b"]));
        let pkg_b = make_pkg_system("b", "1.0.0", Some(vec!["a"]));
        let stub = stub_with(vec![pkg_b]);
        let installed = HashMap::new();

        let result = resolve_deps_with_root(&root_a, &stub, &installed);
        assert!(matches!(result, Err(BallError::DependencyCycle(_))));
    }

    #[test]
    fn test_resolver_tolerates_system_only_cycle() {
        let root_a = make_pkg_system("a", "1.0.0", Some(vec!["b"]));
        let pkg_b = make_pkg_system("b", "1.0.0", Some(vec!["a"]));
        let stub = stub_with(vec![pkg_b]);
        let installed = HashMap::new();

        let result = resolve_deps_with_root(&root_a, &stub, &installed).unwrap();
        let names: Vec<&str> = result.packages.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"a"));
        assert!(names.contains(&"b"));
    }

    #[test]
    fn test_unresolved_dependency_is_reported_not_silently_dropped() {
        let root_a = make_pkg("a", "1.0.0", Some(vec!["missing"]));
        let stub = stub_with(vec![]);
        let installed = HashMap::new();

        let resolved = resolve_deps_with_root(&root_a, &stub, &installed).unwrap();
        assert_eq!(resolved.unresolved, vec!["missing"]);
        assert!(resolved.packages.iter().all(|p| p.name != "missing"));
    }

    #[test]
    fn test_system_virtual_dependency_is_tolerated() {
        let root_a = make_pkg_system("a", "1.0.0", Some(vec!["lib-provider"]));
        let stub = stub_with(vec![]);
        let installed = HashMap::new();

        let resolved = resolve_deps_with_root(&root_a, &stub, &installed).unwrap();
        assert!(resolved.unresolved.is_empty());
        assert!(resolved.packages.iter().all(|p| p.name != "lib-provider"));
    }

    #[test]
    fn test_missing_dep_with_non_system_depender_is_reported() {
        let root_a = make_pkg("a", "1.0.0", Some(vec!["b", "missing"]));
        let pkg_b = make_pkg_system("b", "1.0.0", Some(vec!["missing"]));
        let stub = stub_with(vec![pkg_b]);
        let installed = HashMap::new();

        let resolved = resolve_deps_with_root(&root_a, &stub, &installed).unwrap();
        assert!(resolved.unresolved.contains(&"missing".to_string()));
        assert!(resolved.packages.iter().all(|p| p.name != "missing"));
    }

    #[test]
    fn test_dependency_debug() {
        let dep = Dependency {
            name: "test".to_string(),
            constraint: VersionReq::STAR,
            optional: false,
        };
        let debug = format!("{:?}", dep);
        assert!(debug.contains("test"));
    }

    #[test]
    fn test_resolve_result_debug() {
        let result = ResolveResult {
            packages: vec![],
            unresolved: vec![],
        };
        let debug = format!("{:?}", result);
        assert!(debug.contains("packages"));
    }

    #[test]
    fn test_parse_version_flexible_standard() {
        let v = parse_version_flexible("1.2.3").unwrap();
        assert_eq!(v, Version::new(1, 2, 3));
    }

    #[test]
    fn test_parse_version_flexible_two_part() {
        let v = parse_version_flexible("1.21").unwrap();
        assert_eq!(v, Version::new(1, 21, 0));
    }

    #[test]
    fn test_parse_version_flexible_debian_epoch() {
        let v = parse_version_flexible("2:1.21-76").unwrap();
        assert_eq!(v, Version::new(1, 21, 0));
    }

    #[test]
    fn test_parse_version_flexible_debian_revision() {
        let v = parse_version_flexible("1.21.76-2").unwrap();
        assert_eq!(v, Version::new(1, 21, 76));
    }

    #[test]
    fn test_parse_version_flexible_three_part() {
        let v = parse_version_flexible("8.2.2637").unwrap();
        assert_eq!(v, Version::new(8, 2, 2637));
    }

    #[test]
    fn test_parse_version_flexible_zero_padded_segment() {
        let v = parse_version_flexible("8.1.0875").unwrap();
        assert_eq!(v, Version::new(8, 1, 875));
    }

    #[test]
    fn test_parse_version_flexible_debian_epoch_with_zero_padding() {
        let v = parse_version_flexible("2:8.1.0875-5ubuntu2").unwrap();
        assert_eq!(v, Version::new(8, 1, 875));
    }

    #[test]
    fn test_parse_version_flexible_all_zero_segment() {
        let v = parse_version_flexible("1.00.0").unwrap();
        assert_eq!(v, Version::new(1, 0, 0));
    }

    #[test]
    fn test_strip_leading_zeros() {
        assert_eq!(strip_leading_zeros("0875"), "875");
        assert_eq!(strip_leading_zeros("000"), "0");
        assert_eq!(strip_leading_zeros("0"), "0");
        assert_eq!(strip_leading_zeros("12"), "12");
    }

    #[test]
    fn test_parse_version_flexible_invalid() {
        assert!(parse_version_flexible("not-a-version").is_none());
    }

    #[test]
    fn test_parse_version_flexible_epoch_two_part() {
        let v = parse_version_flexible("2:1.21").unwrap();
        assert_eq!(v, Version::new(1, 21, 0));
    }

    #[test]
    fn test_parse_version_flexible_debian_dfsg() {
        let v = parse_version_flexible("1:1.3.dfsg+really1.3.1-1+b1").unwrap();
        assert_eq!(v, Version::new(1, 3, 0));
    }
}
