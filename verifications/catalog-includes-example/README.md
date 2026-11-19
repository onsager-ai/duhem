# Shared catalog from an independent scope

`scoped/duhem.yml` runs as its own root but imports `shared.yml` from the
nearest enclosing manifest's directory. The shared sign-in flow uses the
shared page locators against a real browser page; the check asserts the
heading becomes visible after sign-in.

From the repository root:

```sh
duhem validate verifications/catalog-includes-example/scoped/duhem.yml
duhem run verifications/catalog-includes-example/scoped/duhem.yml
```

The enclosing manifest supplies only a directory anchor: its verification
list and provisioning are never inherited. The scope may declare its own
provisioning and profiles. Catalog files accept only `pages:` and `flows:`;
ordinary string includes still resolve beside their declaring file.
