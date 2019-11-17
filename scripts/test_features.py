#!/usr/bin/env python3
import glob
import os.path
import subprocess
import sys
import toml


def load_features(path):
    """
    Loads a Cargo.toml file and extracts all features from it.
    :param path: The path to the Cargo.toml
    :return: A list of features this package has
    """

    with open(path, 'r') as f:
        config = toml.load(f)
        if 'features' not in config:
            return []

        return list(config['features'].keys())


def get_pkg_path(pkgid):
    result = subprocess.run(['cargo', 'pkgid', pkgid], check=True, capture_output=True)
    return result.stdout.decode().replace('file://', '').split('#')[0]


def build_with_features(path, features=[], mode='check'):
    subprocess.run(['cargo', mode, '--quiet', '--features' if len(features) > 0 else '--no-default-features']
                   + features, cwd=path, check=True)


def print_err(*args, **kwargs):
    print(*args, file=sys.stderr, **kwargs)


def test_features(path, mode):
    try:
        features = load_features(os.path.join(path, 'Cargo.toml'))
    except Exception as e:
        print_err('Could not load Cargo.toml and identify features')
        print_err(e)
        sys.exit(1)

    print('Identified the following features:')
    print(', '.join(features))

    print('Start building in `{}` mode with each feature separately enabled.'.format(mode))
    # No features enabled
    print('1/{}: no features'.format(len(features)+1))
    try:
        build_with_features(path, mode=mode)
    except subprocess.CalledProcessError as e:
        print_err('Failed to build without features for package at {}'.format(path))
        print_err(e.stderr)
        sys.exit(2)

    # Individual features
    for i, feature in enumerate(features):
        print('{}/{}: {}'.format(i+2, len(features)+1, feature))

        try:
            build_with_features(path, [feature], mode=mode)
        except subprocess.CalledProcessError as e:
            print_err('Failed to build feature {} for package at {}'.format(feature, path))
            print_err(e.stderr)
            sys.exit(2)


if __name__ == '__main__':
    if len(sys.argv) < 2:
        print('Usage is:')
        print('{} pkgid [mode] - Tries building the package with name pkgid.'.format(sys.argv[0]))
        print('    Each feature of the package is build separately.')
        print('    The default mode is `check`.')
        print('    If pkgid is set to `--workspace`, it instead runs this script on all packages in the workspace.')

    pkgid = sys.argv[1]
    mode = sys.argv[2] if len(sys.argv) > 2 else 'check'

    if pkgid == '--workspace':
        for filename in glob.iglob('**/Cargo.toml', recursive=True):
            # Skip main project
            if filename == 'Cargo.toml':
                continue

            path = filename.replace('Cargo.toml', '')
            print('Running build for path: `{}`'.format(path))
            test_features(path, mode)
    else:
        try:
            path = get_pkg_path(pkgid)
        except subprocess.CalledProcessError as e:
            print_err('Could not identify package with id {}'.format(pkgid))
            print_err(e.stderr)
            sys.exit(1)

        test_features(path, mode)
