Small hand-authored examples of documented producer JUnit XML shapes.
These are fixtures, not records of actual test runs.

nextest.xml: nextest JUnit configuration/output, binary id as suite/classname.
https://nexte.st/docs/machine-readable/junit/
pytest.xml: pytest --junitxml, including setup errors and pytest.skip messages.
https://docs.pytest.org/en/stable/how-to/output.html#creating-junitxml-format-files
jest-junit.xml: jest-junit suite/classname/name templates and skipped cases.
https://github.com/jest-community/jest-junit#configuration
playwright.xml: Playwright JUnit reporter, project hostname and CDATA failures.
https://playwright.dev/docs/test-reporters#junit-reporter
go-junit-report.xml: Go package suite/classname, failure and skipped messages.
https://github.com/jstemmer/go-junit-report

pass.xml and malformed.xml exercise successful ingestion and malformed evidence.
