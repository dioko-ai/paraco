# Sandbox probe

Run `paraco sandbox-probe --port 3000` and open the displayed address. This command supplies independently collected host containment evidence to the example. The report shows guest checks, host checks, and limitations separately. A PASS applies to the specific probes performed; it is not a guarantee against every possible escape.

Running `paraco run examples/sandbox-probe` starts the example without trusted host evidence, so its overall result remains UNVERIFIED unless a guest probe finds a failure. The optional fixture field accepts an HTTP URL on 127.0.0.1 for a server you control. A response is a failure; a request error or timeout is inconclusive.
