import type { ClaimScope } from "../api";

export function ClaimScopeNotice({ scope }: { scope?: ClaimScope }) {
  if (!scope) return null;
  return (
    <section aria-label="Verified scope" className="space-y-2 border-y py-4 text-sm" data-testid="claim-scope">
      <h3 className="font-semibold">Verified scope</h3>
      <p>{scope.criteria.executed} of {scope.criteria.authored} criteria and {scope.checks.executed} of {scope.checks.authored} checks have recorded outcomes.</p>
      <p className="text-muted-foreground">Selected: {scope.criteria.selected} criteria, {scope.checks.selected} checks. Recorded outcomes include inconclusive preconditions; they do not establish a passing body.</p>
      {scope.manifest && <p>{scope.manifest.selected} of {scope.manifest.authored} manifest leaves selected.</p>}
      {!!scope.filters?.length && <p>Filter: <code>{scope.filters.join(" OR ")}</code></p>}
      {scope.manifest?.directory && <p>Directory: <code>{scope.manifest.directory}</code></p>}
      {!!scope.excluded?.length && <ul className="list-disc pl-5">{scope.excluded.map((claim) => (
        <li key={`${claim.criterion_id}::${claim.check_id}`}>Excluded <code>{claim.criterion_id}::{claim.check_id}</code>: {claim.reason}</li>
      ))}</ul>}
      {!!scope.manifest?.excluded?.length && <ul className="list-disc pl-5">{scope.manifest.excluded.map((leaf) => (
        <li key={leaf.verification}>Excluded leaf <code>{leaf.verification}</code>: {leaf.reason}</li>
      ))}</ul>}
    </section>
  );
}
