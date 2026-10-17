# Command-line client (`udm-cli`)

`udm-cli` runs the same commands as the app. By default it starts its own engine on the data folder; with `--control PORT` it talks to a running UDM instead.

```powershell
udm-cli add http://127.0.0.1:8099/files/big.bin --dir .udm-dev\out --connections 8 --wait
udm-cli verify .udm-dev\out\big.bin --sha256 <hex>
udm-cli list
udm-cli exec pause d-000001
udm-cli run --script .udm-dev\script.jsonl
udm-cli --control 7878 list
```

| Option | Meaning |
|---|---|
| `--data-dir DIR` | Settings, queue and resume state. Default: the OS data folder. |
| `--control PORT` | Send requests to a running UDM on `127.0.0.1:PORT`. |
| `--locale id` / `--locale en` | Language of error messages (default `id`). |

- Results are printed as one JSON value per line. Errors go to stderr as a localized sentence followed by the message key, and the exit code is 1.
- `run --script` reads one control request per line (the same JSON as the control port). Empty lines and lines starting with `#` are skipped. Scripts are limited to 8 MiB.
- If the UDM app is already running on the same data folder, `udm-cli` refuses to start a second engine (`error-cli-app-running`); use `--control`.
- `add --wait` waits until the download completes (exit 0) or fails (exit 1). An interrupted wait leaves the download in the queue; it resumes safely later.
