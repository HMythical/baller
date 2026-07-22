#!/usr/bin/env python3
"""
Artifact Detection Script

Detects forbidden files and directories that should not be committed to the repository.
Used by GitHub Actions to prevent build artifacts and secrets from being committed.
"""

import os
import sys
from pathlib import Path
from typing import List, Tuple


FORBIDDEN_DIRECTORIES = [
    "target",
    "build/linux/dist",
    "build/winbuild/dist",
    ".idea",
    ".vscode",
    ".claude",
    "node_modules",
    "__pycache__",
]

FORBIDDEN_PATTERNS = {
    "compiled_binaries": ["*.exe", "*.dll", "*.so", "*.dylib"],
    "debug_symbols": ["*.pdb"],
    "debug_directories": ["*.dSYM"],
    "logs_temp": ["*.log", "*.tmp"],
    "secrets": [".env", ".env.*", "*.key", "*.pem"],
    "other": ["credentials", "desktop.ini"],
}


def find_forbidden_directories(repo_root: Path) -> List[str]:
    """
    Find forbidden directories in the repository.
    
    Args:
        repo_root: Path to the repository root directory
        
    Returns:
        List of forbidden directories found
    """
    found = []
    
    for dir_pattern in FORBIDDEN_DIRECTORIES:
        dir_path = repo_root / dir_pattern
        if dir_path.exists() and dir_path.is_dir():
            found.append(dir_pattern)
            print(f"✗ Found forbidden directory: {dir_pattern}")
    
    return found


def find_forbidden_files(repo_root: Path) -> List[str]:
    """
    Find forbidden files in the repository.
    
    Args:
        repo_root: Path to the repository root directory
        
    Returns:
        List of forbidden files found
    """
    found = []
    
    for category, patterns in FORBIDDEN_PATTERNS.items():
        for pattern in patterns:
            # Handle special cases
            if pattern in ["credentials", "desktop.ini"]:
                file_path = repo_root / pattern
                if file_path.exists() and file_path.is_file():
                    found.append(pattern)
                    print(f"✗ Found forbidden file: {pattern}")
            else:
                # Use glob pattern matching
                for file_path in repo_root.rglob(pattern):
                    # Skip .git directory
                    if ".git" in file_path.parts:
                        continue
                    
                    # Convert to relative path for display
                    rel_path = file_path.relative_to(repo_root)
                    found.append(str(rel_path))
                    print(f"✗ Found forbidden file: {rel_path}")
    
    return found


def detect_artifacts(repo_root: Path) -> bool:
    """
    Detect forbidden files and directories in the repository.
    
    Args:
        repo_root: Path to the repository root directory
        
    Returns:
        True if no forbidden items found, False otherwise
    """
    print("Checking for forbidden directories...")
    print()
    
    forbidden_dirs = find_forbidden_directories(repo_root)
    
    print()
    print("Checking for forbidden file patterns...")
    print()
    
    forbidden_files = find_forbidden_files(repo_root)
    
    print()
    
    if forbidden_dirs or forbidden_files:
        total = len(forbidden_dirs) + len(forbidden_files)
        print(f"ERROR: Forbidden items found: {total}")
        print("These items should not be committed to the repository.")
        return False
    
    print("✓ No forbidden items found")
    return True


def main() -> int:
    """Main entry point for the script."""
    # Get repository root from environment or current directory
    repo_root_str = os.environ.get("GITHUB_WORKSPACE", os.getcwd())
    repo_root = Path(repo_root_str)
    
    print("=" * 50)
    print("Artifact Detection")
    print("=" * 50)
    print()
    
    success = detect_artifacts(repo_root)
    
    print()
    print("=" * 50)
    
    if success:
        print("Result: PASSED")
    else:
        print("Result: FAILED")
    
    print("=" * 50)
    
    return 0 if success else 1


if __name__ == "__main__":
    sys.exit(main())
