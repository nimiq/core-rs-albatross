from collections import defaultdict
import re
import functools
import operator
import os
import os.path
import toml
import sys

dependency = re.compile(r'^\s*use\s*([^;\]]+)\s*;\s*$', re.MULTILINE)
compressed_dependency = re.compile(r'([\w:]+)(\{(.*)\})?')
extern_crate = re.compile(
    r'^\s*(#\[macro_use\])?\s*extern\s+crate\s+(\w+)(\s+as\s+(\w+))?\s*;\s*$', re.MULTILINE)


def load_cargo_toml(path):
    """
    Loads a Cargo.toml file and extracts dependency information from it.
    Local names are already mapping `-` to `_`.
    :param path: The path to the Cargo.toml
    :return: A dict in the following form (or None if no crate name is found):
    {
        "name": <crate name>,
        "dependencies": [
            {
                "name": <real crate name>,
                "local_name": <local crate name if renamed>,
                "version": <version (may be None)>,
            },
            ...
        ],
        "dev-dependencies": [
            ...
        ],
    }
    """
    def load_dependencies(deps):
        return [{
            'name': cfg['package'] if isinstance(cfg, dict) and 'package' in cfg else dep,
            'local_name': dep.replace('-', '_'),
            'version': cfg if isinstance(cfg, str) else cfg['version'] if 'version' in cfg else None,
        } for (dep, cfg) in deps.items()]

    with open(path, 'r') as f:
        config = toml.load(f)
        if 'package' not in config or 'name' not in config['package']:
            return None

        name = config['package']['name']
        dependencies = load_dependencies(
            config['dependencies']) if 'dependencies' in config else []
        dev_dependencies = load_dependencies(
            config['dev-dependencies']) if 'dev-dependencies' in config else []

        return {
            'name': name,
            'dependencies': dependencies,
            'dev-dependencies': dev_dependencies,
        }


def format_path(path, modules_only):
    """
    Formats Rust paths.
    :param path: A Rust path delimited by `::`
    :param modules_only: Whether to cut off the last component of the path
    :return:
    """
    if modules_only:
        path = path.split('::')
        if len(path[-1]) == 0 or path[-1][0].isupper():
            path.pop()
        path = '::'.join(path)
    return path


def split_first_level(dependency):
    """
    Internal function to split nested dependencies.
    :param dependency:
    :return:
    """
    dependencies = []
    beginning = 0
    level = 0
    for i in range(len(dependency)):
        if dependency[i] == ',' and level == 0:
            dependencies.append(dependency[beginning:i].strip())
            beginning = i + 1
        elif dependency[i] == '{':
            level += 1
        elif dependency[i] == '}':
            level -= 1
    dependencies.append(dependency[beginning:].strip())
    return dependencies


def uncompress_dependencies(dependency):
    """
    Uncompresses nested dependencies.
    :param dependency:
    :return:
    """
    m = compressed_dependency.match(dependency)
    if m is None:
        print('Dependency did not match:', dependency)
        return []

    main = m.group(1)
    if m.group(2):
        dependencies = [uncompress_dependencies(
            main + sub.strip()) for sub in split_first_level(m.group(3))]
        return functools.reduce(operator.iconcat, dependencies, [])
    else:
        return [main]


def absolute_path(dependency, current_path):
    """
    Refactors `self::` and `super::` uses to absolute paths.
    :param dependency:
    :param current_path:
    :return:
    """
    path_elements = dependency.split('::')

    if path_elements[0] == 'self':
        path_elements[0] = current_path
    elif path_elements[0] == 'super':
        # count number of supers
        count = 0
        while path_elements[0] == 'super':
            path_elements.pop(0)
            count += 1
        path_elements.insert(0, '::'.join(current_path.split('::')[:-count]))
    else:
        return dependency

    return '::'.join(path_elements)


def path_from_filename(filename, source_dir):
    """
    Returns the `crate::` Rust path from a filename.
    :param filename:
    :param source_dir:
    :return:
    """
    filename = filename.replace(source_dir, '').strip(os.sep)
    path = filename.split(os.sep)

    if path[-1] == 'mod.rs':
        path.pop()
    elif path[-1].endswith('.rs'):
        path[-1] = path[-1][:-3]

    return '::'.join(['crate'] + path)


def estimate_crate_name_from_path(path, source_dir):
    path_elements = path.split('::')

    d = []
    name = ''
    for element in path_elements:
        if element == 'crate':
            continue

        d.append(element)
        sys_dir = os.path.join(source_dir, *d)
        if os.path.isdir(sys_dir) and os.path.isfile(os.path.join(sys_dir, 'Cargo.toml')):
            name = os.path.sep.join(d)

    return name


def find_extern_crates(text):
    """
    Returns extern crate information for a string.
    :param text: A text string.
    :return: A dict in the form of (macro_use and externs are distinct, both use local names):
    {
        "externs": [<crate>, ...],
        "macro_use": [<crate>, ...],
        "renames": {
            <local_name>: <real_name>,
            ...
        },
    }
    """
    externs = []
    macro_use = []
    renames = {}
    extern_crates = extern_crate.findall(text)
    for (macro, real_name, _, local_name) in extern_crates:
        if local_name == '':
            local_name = real_name
        else:
            renames[local_name] = real_name

        if macro == '':
            externs.append(local_name)
        else:
            macro_use.append(local_name)

    return {
        'externs': externs,
        'macro_use': macro_use,
        'renames': renames,
    }


def list_dependencies(filename, source):
    """
    Returns a list of all dependencies (including `macro_use` externs) and renames used in the given file.
    :param filename: File to scan
    :param source: Source directory required for absolute path
    :return: A dict in the form:
    {
        'dependencies': [<list of dependencies (always absolute paths, local names)>],
        'renames': {
            <local_name>: <real_name>,
            ...
        },

    }
    """
    current_path = path_from_filename(filename, source)

    with open(filename, 'rt') as f:
        content = f.read()

    extern_crates = find_extern_crates(content)

    dependencies = map(uncompress_dependencies, dependency.findall(content))
    dependencies = functools.reduce(
        operator.iconcat, dependencies, extern_crates['macro_use'])
    dependencies = map(lambda x: absolute_path(x, current_path), dependencies)
    return {
        'dependencies': dependencies,
        'renames': extern_crates['renames'],
    }


def list_dependencies_recursive(directory, source=None):
    """
    Scans files recursively, returning a dict of dependencies.
    :param directory: Directory to scan
    :param source: Source directory required for absolute path (may be None on the first call)
    :return: A dict in the form:
    {
        <filename as path>: {
            'dependencies': [<dependency>, ...],
            'renames': {
                <local_name>: <real_name>,
                ...
            },
        },
        ...
    }
    """
    dependencies = {}
    if source is None:
        source = directory

    for file in os.listdir(directory):
        filename = os.path.join(directory, file)
        if os.path.isdir(filename):
            dependencies.update(list_dependencies_recursive(filename, source))
        elif filename.endswith('.rs'):
            dependencies[path_from_filename(
                filename, source)] = list_dependencies(filename, source)

    return dependencies


def load_external_dependencies(path):
    """
    Extracts external dependencies from code files in the path.
    This considers only non `crate::`, `self::`, and `super::` uses.
    Moreover, it includes `extern crate` statements with a `#[macro_use]`.
    Renames with `extern crate` are considered, but overapproximated (i.e. all renames apply for the whole path).
    Note: This does not check for `mod` statements, but considers all Rust files relevant.
    :param path: The path to recursively scan for dependencies.
    :return: A set in the form of:
    {
        <dependency_name>, ...
    }
    """
    def external_crate_name(rust_path):
        elements = rust_path.split('::')
        if elements[0] not in ('crate', 'super', 'self'):
            return elements[0]
        return None

    dependency_tree = list_dependencies_recursive(path)
    renames = {}
    dependencies = set()

    for (filename, info) in dependency_tree.items():
        renames.update(info['renames'])
        curated_dependencies = filter(lambda x: x is not None, map(
            external_crate_name, info['dependencies']))
        dependencies.update(curated_dependencies)

    return set([renames[name] if name in renames else name for name in dependencies])


def to_dot_graph(dependencies):
    graph = 'digraph {\n'

    nodes = set(dependencies.keys())
    for file in dependencies:
        nodes.update(dependencies[file])

    for node in nodes:
        graph += '    "{}" [label="{}"];\n'.format(node, node)

    for file in dependencies:
        for dep in dependencies[file]:
            f = ''
            if dep in dependencies and file in dependencies[dep]:
                f = ' [color=red]'  # connection in both directions
            graph += '    "{}" -> "{}"{};\n'.format(file, dep, f)

    graph += '}'
    return graph


assert split_first_level('sync::Arc, time::{Duration, Instant}, test::B') == ['sync::Arc', 'time::{Duration, Instant}',
                                                                              'test::B']

assert uncompress_dependencies('std::sync::C') == ['std::sync::C']
assert uncompress_dependencies('std::sync::{a::B}') == ['std::sync::a::B']
assert uncompress_dependencies('std::sync::{a::B, c::D}') == [
    'std::sync::a::B', 'std::sync::c::D']
assert uncompress_dependencies('std::sync::{a::B, c::D, e::F}') == ['std::sync::a::B', 'std::sync::c::D',
                                                                    'std::sync::e::F']
assert uncompress_dependencies('std::sync::{a::B, c::{D, E}, e::F}') == ['std::sync::a::B', 'std::sync::c::D',
                                                                         'std::sync::c::E', 'std::sync::e::F']

assert absolute_path('crate::test::T', 'crate::test') == 'crate::test::T'
assert absolute_path('ext::test::T', 'crate::test') == 'ext::test::T'
assert absolute_path('self::T', 'crate::test') == 'crate::test::T'
assert absolute_path('super::T', 'crate::test::sub') == 'crate::test::T'
assert absolute_path('super::super::T', 'crate::test::sub') == 'crate::T'

assert path_from_filename('src/consensus/consensus.rs',
                          'src') == 'crate::consensus::consensus'
assert path_from_filename('src/consensus/consensus.rs',
                          'src/') == 'crate::consensus::consensus'
assert path_from_filename('src/consensus/mod.rs', 'src') == 'crate::consensus'
assert path_from_filename('src/consensus/test/mod.rs',
                          'src') == 'crate::consensus::test'


# print(to_dot_graph(format_dependencies(list_dependencies_recursive('src'))))

def find_unused_externs(path, dependencies):
    """
    Given a path and a list of expected dependencies,
    this returns the list of dependencies that is not used in the path.
    :param path: The path to scan for used dependencies
    :param dependencies: The dependencies expected to be used.
    :return: A set of unused dependencies.
    """
    used_dependencies = load_external_dependencies(path)
    return set(dependencies) - used_dependencies


def analyze_crate_dependencies(path, data=None):
    """
    Checks for unused externs and gives suggestions for both src and tests/benches directories.
    :param path: The path to the crate to analyze
    :param data: Optional crate data loaded from `load_cargo_toml`
    :return: A dict in the form:
    {
        'name': <crate name>,
        'remove': [<crate>, ...],
        'move-to-dev': [<crate>, ...],
    }
    """
    def get_dependencies(dependencies):
        deps = []
        renames = {}
        for dep in dependencies:
            deps.append(dep['local_name'])
            if dep['local_name'] != dep['name']:
                renames[dep['local_name']] = dep['name']
        return deps, renames

    data = load_cargo_toml(os.path.join(path, 'Cargo.toml'))
    local_dependencies, renames = get_dependencies(data['dependencies'])
    local_dev_dependencies, dev_renames = get_dependencies(
        data['dev-dependencies'])
    renames.update(dev_renames)

    unused_dependencies = find_unused_externs(
        os.path.join(path, 'src'), local_dependencies)
    unused_dev_dependencies = unused_dependencies
    if os.path.isdir(os.path.join(path, 'tests')):
        unused_dev_dependencies = find_unused_externs(os.path.join(path, 'tests'),
                                                      local_dependencies + local_dev_dependencies)
    if os.path.isdir(os.path.join(path, 'benches')):
        unused_dev_dependencies = unused_dev_dependencies.intersection(find_unused_externs(os.path.join(path, 'benches'),
                                                                                           local_dependencies + local_dev_dependencies))

    # We should move those dependencies that are unused in src but used in the tests path
    to_move = unused_dependencies.intersection(
        set(local_dependencies) - unused_dev_dependencies)
    to_remove = (unused_dependencies - to_move) | unused_dev_dependencies.intersection(
        set(local_dev_dependencies) - set(local_dependencies))

    to_move = [renames[crate]
               if crate in renames else crate for crate in to_move]
    to_remove = [renames[crate]
                 if crate in renames else crate for crate in to_remove]
    return {
        'name': data['name'],
        'remove': to_remove,
        'move-to-dev': to_move,
    }


def analyze_versions(crate_data):
    """
    Analyzes the set versions across crates and returns non-matching versions.
    :param crate_data: A list of data items from `load_cargo_toml`
    :return: A dict in the form holding those dependencies with inconsistent versioning:
    {
        <dependency>: {
            <crate>: <version>,
            ...
        },
    }
    """
    def get_major(semver):
        if semver is None:
            return None
        digits = semver.lstrip("^").split(".")
        if digits[0] != "0":
            return digits[0]
        else:
            return "0.{}".format(digits[1])
    dependencies = defaultdict(dict)
    versions = defaultdict(set)
    # Fill datastructure first.
    for data in crate_data:
        for dependency in data['dependencies'] + data['dev-dependencies']:
            dependencies[dependency['name']][data['name']] = get_major(dependency['version'])
            versions[dependency['name']].add(get_major(dependency['version']))

    for (dependency, version_set) in versions.items():
        if len(version_set) == 1:
            dependencies.pop(dependency)

    return dependencies


def find_crates(path):
    """
    Recursively finds all sub-crates.
    :param path: The path to start searching in
    :return: A list of tuples [(<path>, <crate data>), ...].
    """
    def find_crates_recursive(directory):
        crates = []

        # Check current path
        cargo_toml = os.path.join(directory, 'Cargo.toml')
        if os.path.isfile(cargo_toml):
            data = load_cargo_toml(cargo_toml)
            if data is not None:
                crates.append((directory, data))

        # Then check sub directories
        for file in os.listdir(directory):
            filename = os.path.join(directory, file)
            # We are only interested in directories
            if not os.path.isdir(filename):
                continue
            crates.extend(find_crates_recursive(filename))

        return crates

    return find_crates_recursive(path)


if __name__ == '__main__':
    if len(sys.argv) < 2 or sys.argv[1] not in ('versions', 'unused'):
        print('Possible arguments are:')
        print(
            '    {} versions [path] - Finds inconsistent versions of dependencies for all crates in path'.format(sys.argv[0]))
        print(
            '    {} unused [path] - Finds unused dependencies for all crates in path'.format(sys.argv[0]))

    cmd = sys.argv[1] if len(sys.argv) > 1 else 'help'
    path = sys.argv[2] if len(sys.argv) > 2 else '.'

    if cmd == 'versions':
        crates = map(lambda x: x[1], find_crates(path))
        versions = analyze_versions(crates)
        if versions:
            print(
                'The following dependencies have inconsistent versions throughout the crates.')
        for (dependency, versions) in versions.items():
            print('[{}]'.format(dependency))
            for (crate, version) in versions.items():
                print('    {} uses {}'.format(crate, version))
            print()
    elif cmd == 'unused':
        print('The estimation is that the following packages are unused.')
        print('Be aware that this is entirely based on `use` statements and there are other ways to use those!')
        for (path, info) in find_crates(path):
            result = analyze_crate_dependencies(path, info)
            if len(result['remove']) > 0 or len(result['move-to-dev']) > 0:
                print('[{}]'.format(result['name']))
                if len(result['remove']) > 0:
                    print(' - remove: {}'.format(result['remove']))
                if len(result['move-to-dev']) > 0:
                    print(
                        ' - move to dev-dependencies: {}'.format(result['move-to-dev']))
                print()
