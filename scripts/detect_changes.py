#!/usr/bin/env python3
"""
Change Detection Script

Detects which directories changed in the repository for targeted CI.
Used by GitHub Actions to determine which CI workflows to run.
"""

import os
import sys
from pathlib import Path
from typing import Dict, List


# Mapping of directory patterns to CI targets
CHANGE_MAPPINGS = {
    "linux": ["src/**", "build/linux/**", "Cargo.toml", "Cargo.lock"],
    "windows": ["src/**", "build/winbuild/**", "Cargo.toml", "Cargo.lock"],
    "docs": ["docs/**"],
    "workflows": [".github/**"],
}


def get_changed_files() -> List[str]:
    """
    Get list of changed files from GitHub Actions environment.
    
    Returns:
        List of changed file paths
    """
    # Try to get changed files from GitHub Actions environment
    changed_files_str = os.environ.get("CHANGED_FILES", "")
    
    if changed_files_str:
        return [f.strip() for f in changed_files_str.split("\n") if f.strip()]
    
    # Fallback: try to get from git
    try:
        import subprocess
        result = subprocess.run(
            ["git", "diff", "--name-only", "HEAD~1", "HEAD"],
            capture_output=True,
            text=True,
            check=True
        )
        return [f.strip() for f in result.stdout.split("\n") if f.strip()]
    except (subprocess.CalledProcessError, FileNotFoundError):
        return []


def matches_pattern(file_path: str, pattern: str) -> bool:
    """
    Check if a file path matches a pattern.
    
    Args:
        file_path: Path to check
        pattern: Pattern to match against
        
    Returns:
        True if file matches pattern, False otherwise
    """
    # Simple pattern matching (can be enhanced with fnmatch or glob)
    if pattern.endswith("/**"):
        # Directory pattern
        dir_prefix = pattern[:-3]
        return file_path.startswith(dir_prefix)
    elif "*" in pattern:
        # Wildcard pattern
        import fnmatch
        return fnmatch.fnmatch(file_path, pattern)
    else:
        # Exact match
        return file_path == pattern


def detect_changes(changed_files: List[str]) -> Dict[str, bool]:
    """
    Detect which CI targets are affected by changes.
    
    Args:
        changed_files: List of changed file paths
        
    Returns:
        Dictionary mapping CI targets to whether they should run
    """
    results = {}
    
    for target, patterns in CHANGE_MAPPINGS.items():
        should_run = False
        
        for file_path in changed_files:
            for pattern in patterns:
                if matches_pattern(file_path, pattern):
                    should_run = True
                    break
            
            if should_run:
                break
        
        results[target] = should_run
    
    return results


def main() -> int:
    """Main entry point for the script."""
    print("=" * 50)
    print("Change Detection")
    print("=" * 50)
    print()
    
    # Get changed files
    changed_files = get_changed_files()
    
    if not changed_files:
        print("No changed files detected.")
        print("Defaulting to run all CI workflows.")
        print()
        
        # Default to running all workflows
        results = {target: True for target in CHANGE_MAPPINGS.keys()}
    else:
        print(f"Changed files ({len(changed_files)}):")
        for file_path in changed_files:
            print(f"  - {file_path}")
        print()
        
        # Detect changes
        results = detect_changes(changed_files)
    
    # Display results
    print("Change Detection Results:")
    print("-" * 30)
    
    for target, should_run in results.items():
        status = "✓ Run" if should_run else "⏭ Skip"
        print(f"{target}: {status}")
    
    print()
    print("=" * 50)
    
    # Set outputs for GitHub Actions
    github_output = os.environ.get("GITHUB_OUTPUT")
    if github_output:
        with open(github_output, "a") as f:
            for target, should_run in results.items():
                f.write(f"{target}={'true' if should_run else 'false'}\n")
    
    # Also print in GitHub Actions format
    print("Setting outputs:")
    for target, should_run in results.items():
        print(f"  {target}={'true' if should_run else 'false'}")
    
    print("=" * 50)
    
    return 0


if __name__ == "__main__":
    sys.exit(main())
