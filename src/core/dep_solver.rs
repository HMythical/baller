use std::collections::{HashMap, HashSet, VecDeque};

use semver::{Version, VersionReq};

use crate::core::package::Package;
use crate::core::registry::RegistryClient;
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
}

pub fn resolve_deps(
    root_name: &str,
    registry: &RegistryClient,
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
    registry: &RegistryClient,
    installed: &HashMap<String, String>,
) -> Result<ResolveResult, BallError> {
    resolve_from(&root.name, Some(root), registry, installed)
}

fn resolve_from(
    root_name: &str,
    pinned_root: Option<&Package>,
    registry: &RegistryClient,
    installed: &HashMap<String, String>,
) -> Result<ResolveResult, BallError> {
    let mut resolved: HashMap<String, Package> = HashMap::new();
    let mut graph: HashMap<String, Vec<String>> = HashMap::new();
    let mut constraints: HashMap<String, Vec<(String, VersionReq)>> = HashMap::new();
    let mut queue: VecDeque<String> = VecDeque::new();

    queue.push_back(root_name.to_string());

    while let Some(name) = queue.pop_front() {
        if resolved.contains_key(&name) {
            continue;
        }

        if let Some(root) = pinned_root {
            if name == root.name {
                resolved.insert(name.clone(), root.clone());
                enqueue_deps(root, &mut queue, &resolved, &mut constraints, &mut graph);
                continue;
            }
        }

        if let Some(installed_ver) = installed.get(&name) {
            let pkg = match registry.fetch_package(&name) {
                Ok(p) => p,
                Err(BallError::PackageNotFound(_)) => {
                    // Skip unresolvable dependencies (e.g., Debian virtual packages)
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

            if let Some(dep_cs) = constraints.get(&name) {
                for (from_pkg, c) in dep_cs {
                    if !c.matches(&parsed_ver) {
                        return Err(BallError::VersionConflict(format!(
                            "installed version {} of '{}' does not satisfy constraint '{}' required by '{}'",
                            installed_ver, name, c, from_pkg
                        )));
                    }
                }
            }

            resolved.insert(name.clone(), pkg.clone());
            enqueue_deps(&pkg, &mut queue, &resolved, &mut constraints, &mut graph);
            continue;
        }

        let pkg = match registry.fetch_package(&name) {
            Ok(p) => p,
            Err(BallError::PackageNotFound(_)) => {
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

        if let Some(dep_cs) = constraints.get(&name) {
            for (from_pkg, c) in dep_cs {
                if !c.matches(&parsed_ver) {
                    return Err(BallError::VersionConflict(format!(
                        "version {} of '{}' does not satisfy constraint '{}' required by '{}'",
                        pkg.version, name, c, from_pkg
                    )));
                }
            }
        }

        resolved.insert(name.clone(), pkg.clone());
        enqueue_deps(&pkg, &mut queue, &resolved, &mut constraints, &mut graph);
    }

    // Skip cycle detection for system packages — Debian/RPM commonly have
    // mutual dependencies (e.g. libc6 <-> libgcc-s1) that are handled by
    // native package managers.
    let _ = detect_cycles(&graph);
    let order = topological_sort(&graph)?;

    let mut packages = Vec::new();
    for name in &order {
        if let Some(pkg) = resolved.remove(name) {
            packages.push(pkg);
        }
    }

    Ok(ResolveResult { packages })
}

fn enqueue_deps(
    pkg: &Package,
    queue: &mut VecDeque<String>,
    resolved: &HashMap<String, Package>,
    constraints: &mut HashMap<String, Vec<(String, VersionReq)>>,
    graph: &mut HashMap<String, Vec<String>>,
) {
    let deps = parse_dependencies(pkg);
    let dep_names: Vec<String> = deps
        .iter()
        .filter(|d| !d.optional)
        .map(|d| d.name.clone())
        .collect();

    graph.insert(pkg.name.clone(), dep_names.clone());

    for dep in &deps {
        let entry = constraints.entry(dep.name.clone()).or_default();
        entry.push((pkg.name.clone(), dep.constraint.clone()));

        if !dep.optional && !resolved.contains_key(&dep.name) && !queue.contains(&dep.name) {
            queue.push_back(dep.name.clone());
        }
    }
}

fn parse_dependencies(pkg: &Package) -> Vec<Dependency> {
    let mut result = Vec::new();
    if let Some(deps) = &pkg.dependencies {
        for dep_str in deps {
            result.push(parse_dependency_line(dep_str));
        }
    }
    result
}

pub(crate) fn parse_dependency_line(dep_str: &str) -> Dependency {
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

    let constraint = VersionReq::parse(constraint_str).unwrap_or(VersionReq::STAR);

    Dependency {
        name,
        constraint,
        optional,
    }
}

/// Parse a version string that may use formats other than strict semver.
///
/// Handles Debian epoch prefixes (`2:1.21-76`), Debian/RPM revision suffixes
/// (`1.21-76`), and upstream Fedora release tags (`8.2.2637-20.fc36`).
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

    // Normalize to 3 segments (major.minor.patch) for semver compatibility
    let segments: Vec<&str> = clean.split('.').collect();
    let normalized = match segments.len() {
        0 => return None,
        1 => format!("{}.0.0", segments[0]),
        2 => format!("{}.{}.0", segments[0], segments[1]),
        _ => format!("{}.{}.{}", segments[0], segments[1], segments[2]),
    };

    Version::parse(&normalized).ok()
}

pub(crate) fn detect_cycles(graph: &HashMap<String, Vec<String>>) -> Result<(), BallError> {
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
    ) -> Result<(), BallError> {
        colors.insert(node, Color::Gray);
        path.push(node);

        if let Some(deps) = graph.get(node) {
            for dep in deps {
                match colors.get(dep.as_str()).unwrap_or(&Color::White) {
                    Color::Gray => {
                        let start = path.iter().position(|n| *n == dep.as_str()).unwrap_or(0);
                        let cycle: Vec<&str> = path[start..].to_vec();
                        return Err(BallError::DependencyCycle(format!(
                            "cycle detected: {} -> {}",
                            cycle.join(" -> "),
                            dep
                        )));
                    }
                    Color::White => visit(dep, graph, colors, path)?,
                    Color::Black => {}
                }
            }
        }

        path.pop();
        colors.insert(node, Color::Black);
        Ok(())
    }

    let nodes: Vec<&str> = graph.keys().map(|s| s.as_str()).collect();
    let mut path = Vec::new();
    for name in nodes {
        if colors.get(name) == Some(&Color::White) {
            visit(name, graph, &mut colors, &mut path)?;
        }
    }

    Ok(())
}

pub(crate) fn topological_sort(
    graph: &HashMap<String, Vec<String>>,
) -> Result<Vec<String>, BallError> {
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

    // For system packages, cycles are common (e.g. libc6 <-> libgcc-s1).
    // Return the best available ordering instead of failing.
    Ok(result)
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
        }
    }

    #[test]
    fn test_parse_dependency_line_simple() {
        let dep = parse_dependency_line("foo");
        assert_eq!(dep.name, "foo");
        assert_eq!(dep.constraint, VersionReq::STAR);
        assert!(!dep.optional);
    }

    #[test]
    fn test_parse_dependency_line_with_constraint() {
        let dep = parse_dependency_line("foo >=1.0");
        assert_eq!(dep.name, "foo");
        assert!(!dep.optional);
    }

    #[test]
    fn test_parse_dependency_line_optional() {
        let dep = parse_dependency_line("? foo");
        assert_eq!(dep.name, "foo");
        assert!(dep.optional);
        assert_eq!(dep.constraint, VersionReq::STAR);
    }

    #[test]
    fn test_parse_dependency_line_optional_with_constraint() {
        let dep = parse_dependency_line("? bar >=2.0");
        assert_eq!(dep.name, "bar");
        assert!(dep.optional);
    }

    #[test]
    fn test_parse_dependency_line_trimmed() {
        let dep = parse_dependency_line("  baz  ");
        assert_eq!(dep.name, "baz");
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
        let deps = parse_dependencies(&pkg);
        assert!(deps.is_empty());
    }

    #[test]
    fn test_parse_dependencies_empty_vec() {
        let pkg = make_pkg("test", "1.0", Some(vec![]));
        let deps = parse_dependencies(&pkg);
        assert!(deps.is_empty());
    }

    #[test]
    fn test_parse_dependencies_multiple() {
        let pkg = make_pkg("test", "1.0", Some(vec!["dep1", "? dep2", "dep3 >=2.0"]));
        let deps = parse_dependencies(&pkg);
        assert_eq!(deps.len(), 3);
        assert_eq!(deps[0].name, "dep1");
        assert!(!deps[0].optional);
        assert_eq!(deps[1].name, "dep2");
        assert!(deps[1].optional);
        assert_eq!(deps[2].name, "dep3");
        assert!(!deps[2].optional);
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
        let result = ResolveResult { packages: vec![] };
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
