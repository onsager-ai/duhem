# Named browser sessions

This example runs against a small HTTP application with server-issued cookies,
SQLite users and permissions, and actual login forms. Python 3 and Chromium are
required. The credentials below are local demo accounts only.

```sh
python3 verifications/named-sessions-example/app.py --port 3000 --database /tmp/named-sessions.sqlite
# In another terminal:
duhem run verifications/named-sessions-example/duhem.yml --capture always
```

AC-1 signs admin in, proves user1 is still signed out, grants access as admin,
then signs user1 in and observes Reports. Each name owns a separate browser
context throughout the check; steps execute in the order written. AC-2 seeds
admin from captured setup state and proves a null-valued visitor still redirects.

The browser integration test also reroutes user1 steps to admin as a negative
control: the signed-out Login-heading assertion fails, demonstrating that shared cookies
cannot satisfy the isolation claim. Use a fresh database to reset demo permissions.
