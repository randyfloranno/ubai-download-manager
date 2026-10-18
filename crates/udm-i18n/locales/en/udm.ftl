# UDM — English catalogue (fallback).
## dynamic-prefixes: category- power- status- unit- cmd- after-done- proxy-kind-
## not-keys: error-x proxy-user proxy-pass proxy-authorization proxy-connection

app-title = UDM — Ubai Download Manager

## Download status
status-queued = Queued
status-scheduled = Scheduled
status-probing = Connecting
status-downloading = Downloading
status-paused = Paused
status-paused-disk-full = Paused: disk full
status-paused-disk-write = Paused: disk write error
status-paused-shutdown = Paused (app closed)
status-paused-schedule = Paused by scheduler
status-finalising = Finalising
status-completed = Completed
status-failed = Failed

## Categories
category-documents = Documents
category-video = Video
category-music = Music
category-programs = Programs
category-archives = Archives
category-general = General

## After-completion actions
after-done-none = Do nothing
after-done-exit = Exit UDM
after-done-shutdown = Shut down the computer
after-done-sleep = Sleep
after-done-hibernate = Hibernate

## Proxy kinds
proxy-kind-none = No proxy
proxy-kind-http = HTTP proxy
proxy-kind-socks5 = SOCKS5 proxy

## Notices
download-restarted-remote-changed = The file on the server changed, so the download restarted from the beginning.
notice-resume-state-corrupt = The saved progress was damaged; the download starts again.
notice-history-lines-skipped = { $count } damaged history entries were skipped.
notice-queue-finished = Queue "{ $queue }" has finished.
notice-verify-mismatch = The checksum of { $name } does not match.

## Command errors
error-command-too-many-args = Too many arguments (maximum { $max }).
error-command-unknown-arg = Command { $command } has no argument "{ $arg }".
error-command-bad-arg = Invalid value for argument "{ $arg }" of { $command }.
error-command-missing-arg = Command { $command } needs argument "{ $arg }".
error-command-needs-confirmation = Command { $command } needs confirmation.
error-command-unknown = Unknown command { $command }.
error-command-bad-args = The command arguments must be a JSON object.
error-state-coverage-overlap = The saved progress is corrupt (overlapping ranges).

## Engine errors
error-engine-not-found = Download { $id } does not exist.
error-engine-queue-not-found = Queue "{ $queue }" does not exist.
error-engine-queue-exists = Queue "{ $queue }" already exists.
error-engine-too-many-queues = There are too many queues.
error-engine-queue-main = The main queue cannot be deleted.
error-engine-queue-full = The queue is full.
error-engine-category-not-found = Category "{ $category }" does not exist.
error-engine-batch-too-large = The list has too many addresses.
error-engine-job-not-found = Grabber job { $job } does not exist.
error-engine-cannot-pause = This download cannot be paused now.
error-engine-already-completed = This download has already completed.
error-engine-download-active = Stop the download first.
error-engine-not-completed = This download has not completed yet.
error-engine-file-missing = The file { $name } no longer exists.
error-engine-internal = Internal error. The download state was kept.
error-engine-no-free-name = No free file name for { $name }.
error-engine-create-dir = Cannot create folder { $path }.
error-engine-incomplete = The download is incomplete; the file was not delivered.
error-engine-size-mismatch = Wrong file size: expected { $expected }, got { $actual }.
error-engine-checksum-mismatch = Checksum mismatch: expected { $expected }, got { $actual }.
error-engine-read-file = Cannot read file { $path }.
error-settings-unknown = Unknown setting { $name }.
error-settings-bad-value = Invalid value for setting { $name }.
error-schedule-bad-time = Invalid time "{ $value }" (use HH:MM).
error-schedule-bad-day = Invalid day "{ $value }".

## Network errors
error-net-connect = Could not connect to { $host }.
error-net-timeout = The connection timed out after { $seconds } s.
error-net-tls = Secure connection to { $host } failed.
error-net-tls-untrusted = The certificate of { $host } is not trusted (SHA-256 { $fingerprint }).
error-net-http-status = The server answered with status { $status }.
error-net-too-many-redirects = Too many redirects (more than { $max }).
error-net-redirect-loop = The server redirects in a loop.
error-net-downgrade-refused = Redirect from a secure to an insecure address was refused.
error-net-redirect-invalid = The server sent an invalid redirect.
error-net-header-too-large = The server's response headers are too large (limit { $max }).
error-net-too-many-headers = The server sent too many headers (limit { $max }).
error-net-content-encoding = The server compressed the file data ({ $encoding }); this is not supported.
error-net-short-body = The connection ended early after { $received }.
error-net-extra-bytes = The server sent more data than requested.
error-net-bad-content-range = The server sent a wrong byte range.
error-net-remote-changed = The file on the server has changed.
error-net-auth-required = The server requires a user name and password.
error-net-auth-failed = The user name or password was rejected.
error-net-proxy = The proxy refused the connection (code { $code }).
error-net-proxy-auth = The proxy rejected the credentials.
error-net-ftp = FTP server error { $code }: { $text }
error-net-unsupported-scheme = Unsupported address type: { $scheme }.
error-net-invalid-request = The request is invalid.
error-net-io = A network read or write error occurred.
error-net-stream = The data stream was interrupted.
error-net-protocol = The server broke the protocol.
error-net-not-html = The page is not HTML ({ $type }).

## Protocol errors
error-proto-url-too-long = The address is too long ({ $len } characters, maximum { $max }).
error-proto-url-invalid = The address is not valid.
error-proto-url-scheme = Unsupported address type: { $scheme }.
error-proto-url-no-host = The address has no host name.
error-proto-redirect-invalid = The redirect address is not valid.
error-proto-header-value = A header value is too long or invalid.
error-proto-content-range-missing = The server did not send a byte range.
error-proto-content-range-invalid = The server sent an invalid byte range.
error-proto-content-range = Wrong byte range: expected { $expected }, got { $actual }.
error-proto-auth-challenge = The server's login request is invalid.
error-proto-ftp-line-too-long = An FTP reply line is too long (maximum { $max }).
error-proto-ftp-too-many-lines = An FTP reply has too many lines (maximum { $max }).
error-proto-ftp-reply = The FTP server sent an invalid reply.
error-proto-url-list-too-large = The address list is too large ({ $bytes }, maximum { $max }).
error-proto-batch-too-large = Too many addresses ({ $count }, maximum { $max }).
error-proto-batch-pattern = The address pattern is invalid.
error-proto-checksum-invalid = The checksum is invalid (use sha256:<hex> or md5:<hex>).

## Storage errors
error-store-io = Cannot read or write { $path }.
error-store-corrupt = The file { $path } is damaged.
error-store-kind-mismatch = The file { $path } has the wrong content.
error-store-schema-unsupported = The file { $path } comes from a newer UDM version.
error-store-parse = The file { $path } cannot be read.
error-store-too-large = The file { $path } is too large.

## System errors
error-disk-full = The disk is full while writing { $path }.
error-disk-write = Error writing to disk: { $path }.
error-sys-io = File error at { $path }.
error-sys-already-exists = { $path } already exists.
error-sys-path-too-long = The path is too long: { $path }.
error-sys-file-too-large = The file is too large.
error-sys-unsupported = Not supported on this system.
error-sys-clipboard = The clipboard cannot be read.
error-sys-notify = The notification cannot be shown.
error-sys-power = The power action failed.
error-sys-open = Could not open { $path }.
error-sys-not-enough-space = Not enough free space on { $volume }: { $needed } needed, { $available } available (short by { $shortfall }).

## Secret storage errors
error-secrets-unavailable = The system keychain is not available.
error-secrets-backend = The system keychain reported an error.
error-secrets-invalid-ref = Invalid keychain reference.
error-secrets-too-long = The secret is too long.

## Commands (label and description)
cmd-download-add = Add download
cmd-download-add-desc = Add a new download from an address.
cmd-download-pause = Pause
cmd-download-pause-desc = Pause the download; it can be resumed later.
cmd-download-resume = Resume
cmd-download-resume-desc = Continue a paused or failed download.
cmd-download-remove = Remove
cmd-download-remove-desc = Remove the download from the list.
cmd-download-restart = Restart
cmd-download-restart-desc = Download the file again from the beginning.
cmd-download-verify = Verify checksum
cmd-download-verify-desc = Compute the checksum of the finished file.
cmd-download-open = Open
cmd-download-open-desc = Open the finished file with its default program.
cmd-download-open-folder = Open file location
cmd-download-open-folder-desc = Show the file in its folder.
cmd-download-set-connections = Set connections
cmd-download-set-connections-desc = Change the number of connections of a download.
cmd-download-pause-all = Pause all
cmd-download-pause-all-desc = Pause every running download.
cmd-download-resume-all = Resume all
cmd-download-resume-all-desc = Resume every paused download.
cmd-download-remove-completed = Remove completed
cmd-download-remove-completed-desc = Remove all completed downloads from the list.
cmd-queue-start = Start queue
cmd-queue-start-desc = Start processing the queue.
cmd-queue-stop = Stop queue
cmd-queue-stop-desc = Stop processing the queue.
cmd-queue-create = New queue
cmd-queue-create-desc = Create a new queue.
cmd-queue-delete = Delete queue
cmd-queue-delete-desc = Delete a queue; its downloads move to the main queue.
cmd-queue-set-concurrency = Set concurrent downloads
cmd-queue-set-concurrency-desc = How many downloads of the queue run at the same time.
cmd-queue-move-item = Move in queue
cmd-queue-move-item-desc = Move a download to another queue or position.
cmd-schedule-set = Set schedule
cmd-schedule-set-desc = Start and stop a queue at set times.
cmd-schedule-clear = Clear schedule
cmd-schedule-clear-desc = Remove the schedule of a queue.
cmd-limit-set-global = Global speed limit
cmd-limit-set-global-desc = Limit the total download speed (0 = unlimited).
cmd-limit-set-download = Download speed limit
cmd-limit-set-download-desc = Limit the speed of one download (0 = unlimited).
cmd-category-assign = Set category
cmd-category-assign-desc = Put a download into a category.
cmd-category-set-folder = Category folder
cmd-category-set-folder-desc = Choose the folder of a category.
cmd-category-set-extensions = Category file types
cmd-category-set-extensions-desc = Choose which file types belong to a category.
cmd-capture-clipboard-toggle = Clipboard capture
cmd-capture-clipboard-toggle-desc = Watch the clipboard for download links.
cmd-capture-clipboard-text = Add from clipboard
cmd-capture-clipboard-text-desc = Add the download links found in a text.
cmd-capture-browser = Capture from browser
cmd-capture-browser-desc = Add a download captured by the browser extension.
cmd-batch-import = Batch download
cmd-batch-import-desc = Add many downloads from a list of addresses.
cmd-batch-pattern = Batch from pattern
cmd-batch-pattern-desc = Add downloads from an address pattern such as file[1-10].zip.
cmd-grabber-start = Site grabber
cmd-grabber-start-desc = Search a web page for files to download.
cmd-grabber-add-results = Add grabbed files
cmd-grabber-add-results-desc = Add the files found by the site grabber.
cmd-settings-set = Change setting
cmd-settings-set-desc = Change one setting.
cmd-tls-allow-host = Trust certificate
cmd-tls-allow-host-desc = Accept the certificate of one host after checking its details.
cmd-tls-revoke-host = Revoke certificate trust
cmd-tls-revoke-host-desc = Stop accepting the certificate exception of a host.
cmd-proxy-set = Proxy
cmd-proxy-set-desc = Set the proxy server.
cmd-auth-set-site = Site login
cmd-auth-set-site-desc = Save a user name and password for a site.
cmd-auth-remove-site = Remove site login
cmd-auth-remove-site-desc = Forget the saved login of a site.
cmd-history-clear = Clear history
cmd-history-clear-desc = Remove all history entries.
cmd-history-remove = Remove from history
cmd-history-remove-desc = Remove one history entry.

## Control port and command line
error-control-bad-json = The request is not valid JSON.
error-control-bad-request = The request is malformed.
error-control-message-too-large = The message is larger than { $max } bytes.
error-control-no-ui = No window is available for this request.
error-control-unknown-method = Unknown method: { $method }.
error-control-bad-query = Unknown query: { $what }.
error-control-missing-param = Missing parameter: { $param }.
error-control-unauthorized = The request is not authorised.
error-control-forbidden = This method is not allowed on this endpoint.
error-control-busy = UDM is busy; try again shortly.
error-control-connect = Cannot use local port { $port }.
error-control-io = Connection to UDM failed.
error-control-random = Cannot create a secure random token.
error-cmdline-too-long = The command line is longer than { $max } bytes.
error-cmdline-unterminated-quote = A quote is not closed.
error-cmdline-too-many-tokens = The command line has more than { $max } words.
error-cmdline-empty = The command line is empty.
error-cmdline-extra-word = Unexpected word: { $word }.

## Application
error-app-control-refused = The control port is disabled in release builds (set UDM_ALLOW_CONTROL=1).
error-app-sample-unavailable = Sample data is not included in this build.
error-app-runtime = Cannot start the background runtime.
error-app-window = Cannot open the window.
error-app-sample-server = Cannot start the sample server.
error-ui-timeout = The window did not answer in time.
error-ui-screenshot-path = The screenshot path must be a .png file without "..".
error-ui-screenshot-write = Cannot save the screenshot.
error-ui-unknown-dialog = Unknown dialog: { $dialog }.
error-ui-bad-locale = Unknown language: { $locale }.
error-ui-frame = Something went wrong while drawing the window; it has been reset.

## Main window
ui-filter-all = All downloads
ui-filter-unfinished = Unfinished
ui-filter-finished = Finished
ui-section-categories = Categories
ui-section-queues = Queues
ui-queue-main = Main queue
ui-col-name = File name
ui-col-size = Size
ui-col-progress = Progress
ui-col-speed = Speed
ui-col-eta = Time left
ui-col-status = Status
ui-col-added = Added
ui-empty-list = No downloads here yet.
ui-no-selection = Select a download to see its details.
ui-unknown-size = Unknown
ui-status-bar = { $active } active of { $total } · { $speed }
ui-limit-global = Speed limit: { $limit }
ui-limit-global-none = No speed limit
ui-limit-none = None
ui-detail-url = Address
ui-detail-folder = Folder
ui-detail-connections = Connections
ui-detail-limit = Speed limit
ui-detail-downloaded = Downloaded
ui-detail-resume = Resume
ui-resume-yes = Supported
ui-resume-no = Not guaranteed
ui-dangerous-name = Warning: "{ $name }" hides a program behind a document extension.
ui-notice-completed = Download completed: { $name }
ui-verify-match = { $algo } matches: { $hash }
ui-verify-mismatch = { $algo } does NOT match: { $hash }
ui-verify-result = { $algo }: { $hash }
ui-unit-kib-s = {" "}KiB/s
ui-hint-auto = Automatic

## Buttons
ui-btn-add = Add
ui-btn-cancel = Cancel
ui-btn-close = Close
ui-btn-apply = Apply
ui-btn-save = Save

## Dialogs
ui-add-title = Add download
ui-add-url = Address
ui-add-folder = Folder
ui-add-file-name = File name
ui-add-connections = Connections
ui-add-category = Category
ui-add-checksum = Checksum
ui-add-start-now = Start now
ui-settings-title = Settings
ui-settings-language = Language
ui-lang-id = Bahasa Indonesia
ui-lang-en = English
ui-settings-download-dir = Download folder
ui-settings-max-concurrent = Downloads at once
ui-settings-global-limit = Global speed limit
ui-settings-subfolders = Put each category in its own folder
ui-settings-notify = Notify when a download completes
ui-settings-close-to-tray = Closing the window keeps UDM running in the tray
ui-tray-show = Show UDM
ui-tray-quit = Quit
ui-tray-still-running = UDM is still running in the system tray.
ui-schedule-queue = Queue
ui-schedule-start = Start at
ui-schedule-stop = Stop at
ui-schedule-days = Days
ui-schedule-days-hint = mon,tue,… (empty = every day)
ui-schedule-after = When finished
ui-schedule-hint = Times are local, in 24-hour HH:MM format.
ui-batch-hint = One address per line, or a single pattern such as https://example.org/file[1-10].zip
ui-grab-depth = Depth
ui-grab-extensions = File types
ui-grab-same-host = Stay on the same site
ui-grab-start = Search
ui-grab-add = Add found files
ui-grab-running = Searching… { $count } files on { $pages } pages so far
ui-grab-found = Found { $count } files on { $pages } pages
ui-remove-title = Remove download
ui-remove-question = Remove "{ $name }" from the list?
ui-remove-delete-file = Also delete the file from disk
ui-properties-title = Properties
ui-captured-title = Links captured
ui-captured-question = { $count } download links were found on the clipboard. Add them?

## Native messaging host

error-native-too-large = The browser message is too large.
error-native-truncated = The browser message was cut off.
error-native-io = Cannot talk to the browser.
error-native-bad-json = The browser message is not valid JSON.
error-native-bad-message = The browser message has an invalid field: { $field }.
error-native-ipc-unreadable = Cannot read the UDM connection file.
error-native-app-not-running = UDM is not running. Start UDM and try again.
error-native-internal = Internal error in the browser bridge.

## CLI
error-cli-app-running = UDM is running on this data folder. Use --control PORT to talk to it.
error-cli-script-read = Cannot read the script { $path }.
error-cli-script-too-large = The script is larger than { $max } bytes.
error-cli-script-failed = One or more script lines failed.
error-cli-checksum-missing = Give --sha256 or --md5.
error-cli-checksum-mismatch = The checksum of { $name } does not match.
error-cli-download-failed = The download failed.
error-cli-extension-id = { $id } is not a valid extension id (32 letters a–p).
error-cli-host-missing = The native host program was not found: { $path }
error-cli-register-write = Cannot write { $path }.
error-cli-register-reg = Cannot update the Windows registry.

## Browser extension
ext-menu-link = Download link with UDM
ext-menu-media = Download this media with UDM
ext-media-button = Download with UDM
ext-sent = Sent to UDM.
ext-host-missing = UDM is not installed or its browser connector is not registered.
ext-reply-invalid = UDM sent an answer this extension does not understand.
