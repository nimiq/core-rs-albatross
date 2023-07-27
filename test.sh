pkg_names=$(cargo build -p 2>&1 | grep '    ' |  cut -c 5-)
num_pkgs=$(echo $pkg_names | wc -l | xargs)
half_pkgs=$(expr $num_pkgs / 2)

first_half=$(echo $pkg_names | head -n $half_pkgs)
second_half=$(echo $pkg_names | tail -n +$(expr $half_pkgs + 1))

echo $first_half | xargs printf -- '-p %s\n' | xargs cargo nextest run --all-features

# TODO:
# store output somewhere
# load it in other runner
# merge