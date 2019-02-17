import re
import functools
import operator
import os
import os.path

dependency = re.compile(r'^\s*use\s*([^;\]]+)\s*;\s*$', re.MULTILINE)
compressed_dependency = re.compile(r'([\w:]+)(\{(.*)\})?')

modules_only = True
limit_to_crate = False
limit_to_extern = True

def format_path(path):
	if modules_only:
		path = path.split('::')
		if len(path[-1]) == 0 or path[-1][0].isupper():
			path.pop()
		path = '::'.join(path)
	return path

def split_first_level(dependency):
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
	m = compressed_dependency.match(dependency)
	if m is None:
		print('Dependency did not match:', dependency)
		return []

	main = m.group(1)
	if m.group(2):
		dependencies = [uncompress_dependencies(main + sub.strip()) for sub in split_first_level(m.group(3))]
		return functools.reduce(operator.iconcat, dependencies, [])
	else:
		return [main]

def full_path(dependency, current_path):
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

	return '::'.join(path_elements)

def path_from_filename(filename, source_dir):
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

def list_dependencies(filename, source):
	current_path = path_from_filename(filename, source)

	with open(filename, 'rt') as f:
		content = f.read()

	dependencies = map(uncompress_dependencies, dependency.findall(content))
	dependencies = functools.reduce(operator.iconcat, dependencies, [])
	dependencies = map(lambda x: full_path(x, current_path), dependencies)
	return dependencies

def list_dependencies_recursive(directory, source=None):
	dependencies = {}
	if source is None:
		source = directory

	for file in os.listdir(directory):
		filename = os.path.join(directory, file)
		if os.path.isdir(filename):
			dependencies.update(list_dependencies_recursive(filename, source))
		elif filename.endswith('.rs'):
			dependencies[path_from_filename(filename, source)] = list_dependencies(filename, source)

	return dependencies

def format_dependencies(dependencies):
	new_dependencies = {}
	for file in dependencies:
		formatted_file = format_path(file)

		if limit_to_crate and not formatted_file.startswith('crate::'):
			continue

		if limit_to_extern and formatted_file.startswith('crate::'):
			continue

		if formatted_file not in new_dependencies:
			new_dependencies[formatted_file] = set()

		for dep in dependencies[file]:
			if limit_to_crate and not dep.startswith('crate::'):
				continue

			new_dependencies[formatted_file].add(format_path(dep))
	return new_dependencies


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
				f = ' [color=red]' # connection in both directions
			graph += '    "{}" -> "{}"{};\n'.format(file, dep, f)

	graph += '}'
	return graph

assert split_first_level('sync::Arc, time::{Duration, Instant}, test::B') == ['sync::Arc', 'time::{Duration, Instant}', 'test::B']

assert uncompress_dependencies('std::sync::C') == ['std::sync::C']
assert uncompress_dependencies('std::sync::{a::B}') == ['std::sync::a::B']
assert uncompress_dependencies('std::sync::{a::B, c::D}') == ['std::sync::a::B', 'std::sync::c::D']
assert uncompress_dependencies('std::sync::{a::B, c::D, e::F}') == ['std::sync::a::B', 'std::sync::c::D', 'std::sync::e::F']
assert uncompress_dependencies('std::sync::{a::B, c::{D, E}, e::F}') == ['std::sync::a::B', 'std::sync::c::D', 'std::sync::c::E', 'std::sync::e::F']

assert full_path('crate::test::T', 'crate::test') == 'crate::test::T'
assert full_path('ext::test::T', 'crate::test') == 'ext::test::T'
assert full_path('self::T', 'crate::test') == 'crate::test::T'
assert full_path('super::T', 'crate::test::sub') == 'crate::test::T'
assert full_path('super::super::T', 'crate::test::sub') == 'crate::T'

assert path_from_filename('src/consensus/consensus.rs', 'src') == 'crate::consensus::consensus'
assert path_from_filename('src/consensus/consensus.rs', 'src/') == 'crate::consensus::consensus'
assert path_from_filename('src/consensus/mod.rs', 'src') == 'crate::consensus'
assert path_from_filename('src/consensus/test/mod.rs', 'src') == 'crate::consensus::test'

# print(to_dot_graph(format_dependencies(list_dependencies_recursive('src'))))

# List used crates per folder.
def print_extern_crate_uses(path):
	deps = list_dependencies_recursive(path)
	deps_per_crate = {}
	for k in deps:
		local_crate_name = estimate_crate_name_from_path(k, path)
		if local_crate_name not in deps_per_crate:
			deps_per_crate[local_crate_name] = set()

		extern_crate_names = set([use.split('::')[0] for use in deps[k]])
		deps_per_crate[local_crate_name] |= extern_crate_names

	for local_crate in deps_per_crate:
		print(local_crate, list(deps_per_crate[local_crate]))

print_extern_crate_uses('..')