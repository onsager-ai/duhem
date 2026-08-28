#!/usr/bin/env bash
set -euo pipefail

helper="$(cd "$(dirname "$0")/.." && pwd)/scripts/preflight-merge-preview.sh"
test_root="$(mktemp -d "${TMPDIR:-/tmp}/duhem-preflight-test.XXXXXX")"
trap 'rm -rf "$test_root"' EXIT

git_quiet() {
    git -c user.name='Preflight Test' -c user.email='preflight@example.invalid' "$@" >/dev/null
}

make_fixture() {
    local name="$1"
    local bare="$test_root/$name.git"
    local repo="$test_root/$name"
    git_quiet init --bare --initial-branch=main "$bare"
    git_quiet clone "$bare" "$repo"
    (
        cd "$repo"
        printf 'base\n' > shared.txt
        touch branch.txt main.txt
        cat > stage.sh <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [[ -e stage-must-not-run ]]; then
    printf 'stage ran unexpectedly\n' >&2
    exit 70
fi
tokens=$(cat branch.txt main.txt 2>/dev/null | wc -w)
if (( tokens > 6 )); then
    printf 'file-budget violations: merged fixture has %s tokens (budget 6)\n' "$tokens" >&2
    exit 1
fi
printf 'merged-tree stage green\n'
EOF
        chmod +x stage.sh
        git add .
        git_quiet commit -m base
        git_quiet push -u origin main
        git_quiet branch feature
        git symbolic-ref HEAD refs/heads/feature
    )
    printf '%s\n' "$repo"
}

assert_fails_with() {
    local expected="$1"
    shift
    local output status
    set +e
    output="$({ "$@"; } 2>&1)"
    status=$?
    set -e
    if (( status == 0 )) || [[ "$output" != *"$expected"* ]]; then
        printf 'expected failure containing %q; status=%s; output:\n%s\n' "$expected" "$status" "$output" >&2
        exit 1
    fi
}

collision_repo="$(make_fixture collision)"
(
    cd "$collision_repo"
    printf 'one two three four\n' > branch.txt
    git add branch.txt
    git_quiet commit -m branch
    ./stage.sh >/dev/null
)
# Advance main independently, leaving the feature branch honestly green alone.
git_quiet clone "$test_root/collision.git" "$test_root/collision-main"
(
    cd "$test_root/collision-main"
    printf 'five six seven four\n' > main.txt
    git add main.txt
    git_quiet commit -m main
    git_quiet push origin main
)
assert_fails_with 'file-budget violations:' bash -c "cd '$collision_repo' && '$helper' ./stage.sh"

# Reversing which independent addition lands first produces the same failure.
reverse_repo="$(make_fixture collision-reverse)"
(
    cd "$reverse_repo"
    printf 'five six seven four\n' > main.txt
    git add main.txt
    git_quiet commit -m branch
    ./stage.sh >/dev/null
)
git_quiet clone "$test_root/collision-reverse.git" "$test_root/collision-reverse-main"
(
    cd "$test_root/collision-reverse-main"
    printf 'one two three four\n' > branch.txt
    git add branch.txt
    git_quiet commit -m main
    git_quiet push origin main
)
assert_fails_with 'file-budget violations:' bash -c "cd '$reverse_repo' && '$helper' ./stage.sh"

green_repo="$(make_fixture green)"
(
    cd "$green_repo"
    printf 'one two\n' > branch.txt
    git add branch.txt
    git_quiet commit -m branch
)
green_output="$(cd "$green_repo" && "$helper" ./stage.sh 2>&1)"
[[ "$green_output" == *'merged-tree stage green'* ]]

conflict_repo="$(make_fixture conflict)"
(
    cd "$conflict_repo"
    printf 'branch\n' > shared.txt
    touch stage-must-not-run
    git add .
    git_quiet commit -m branch
)
git_quiet clone "$test_root/conflict.git" "$test_root/conflict-main"
(
    cd "$test_root/conflict-main"
    printf 'main\n' > shared.txt
    git add shared.txt
    git_quiet commit -m main
    git_quiet push origin main
)
assert_fails_with 'MERGE PREVIEW CONFLICT' bash -c "cd '$conflict_repo' && '$helper' ./stage.sh"

offline_repo="$(make_fixture offline)"
git -C "$offline_repo" remote set-url origin "$test_root/missing-origin.git"
assert_fails_with 'FETCH FAILED — refusing branch-only gate' bash -c "cd '$offline_repo' && '$helper' ./stage.sh"

printf 'preflight merge-preview tests passed\n'
