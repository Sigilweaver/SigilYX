#!/usr/bin/env bash
# Checks that ci.yml and audit.yml have both completed successfully for a
# given commit before it's tagged and turned into a GitHub Release (which is
# what triggers publish-crate.yml / publish-pypi.yml). GitHub Actions has no
# way for a workflow in one file to `needs:` a job in another file, so this
# has to be enforced here, before `gh release create`, rather than inside the
# publish workflows themselves.
#
# Usage: scripts/check-release-ready.sh [ref]
#   ref defaults to HEAD.

set -euo pipefail

REF="${1:-HEAD}"
REPO="Sigilweaver/SigilYX"

# `^{commit}` peels annotated tags to the commit they point at; git rev-parse
# on an annotated tag alone returns the tag object's own SHA, which never
# matches a workflow run's head SHA.
SHA="$(git rev-parse "${REF}^{commit}")"
echo "Checking release readiness for $REF ($SHA) in $REPO"

check_workflow() {
  local workflow="$1"
  local run_json
  run_json="$(gh run list -R "$REPO" -w "$workflow" -c "$SHA" --json status,conclusion,url -L 1)"

  if [ "$(echo "$run_json" | jq 'length')" -eq 0 ]; then
    echo "FAIL: no $workflow run found for $SHA"
    return 1
  fi

  local status conclusion url
  status="$(echo "$run_json" | jq -r '.[0].status')"
  conclusion="$(echo "$run_json" | jq -r '.[0].conclusion')"
  url="$(echo "$run_json" | jq -r '.[0].url')"

  if [ "$status" != "completed" ]; then
    echo "FAIL: $workflow run for $SHA is not completed (status: $status) - $url"
    return 1
  fi

  if [ "$conclusion" != "success" ]; then
    echo "FAIL: $workflow run for $SHA did not succeed (conclusion: $conclusion) - $url"
    return 1
  fi

  echo "OK: $workflow succeeded for $SHA - $url"
  return 0
}

ci_ok=0
audit_ok=0

check_workflow "ci.yml" || ci_ok=1
check_workflow "audit.yml" || audit_ok=1

if [ "$ci_ok" -eq 0 ] && [ "$audit_ok" -eq 0 ]; then
  echo "Release ready: ci.yml and audit.yml are both green for $SHA"
  exit 0
fi

echo "Release NOT ready: fix the failures above before tagging $SHA"
exit 1
