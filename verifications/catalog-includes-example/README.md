# Shared catalog from an independent scope

`scoped/duhem.yml` runs as its own root but imports
`verifications/catalog-includes-example/shared.yml` relative to the repository
root, found through its `.git` directory or worktree file. Other manifests
do not affect this anchor. The shared sign-in flow uses the
shared page locators against a real browser page; the check asserts the
heading becomes visible after sign-in.

From the repository root:

```sh
duhem validate verifications/catalog-includes-example/scoped/duhem.yml
duhem run verifications/catalog-includes-example/scoped/duhem.yml
```

Other manifests' verification lists and provisioning are never inherited.
The scope may declare its own
provisioning and profiles. Catalog files accept only `pages:` and `flows:`;
ordinary string includes still resolve beside their declaring file.
