# GPX Animator Ride for Android

GPX Animator Ride is the lightweight recorder companion for the native Windows
animator. It deliberately leaves navigation and speed-camera alerts to the
apps already designed for those jobs. Its responsibilities are durable route
capture, standards-compatible GPX export and optional Google Drive handoff.

## Supported Android versions

- Minimum: Android 10 / API 29.
- Compile and target SDK: Android 16 / API 36.
- Primary behavior coverage: Android 13, 14 and 16.
- Android 17 / API 37 is a compatibility test target until the production
  target is deliberately upgraded.

The app requires Google Play services for fused location and Google Drive
authorization. It can still record and export locally when Drive is not
connected or the network is unavailable.

## Build

Install JDK 17 or newer and Android SDK Platform 36, then run from the
repository root:

```powershell
Push-Location mobile-android
$env:ANDROID_HOME = Join-Path $env:LOCALAPPDATA "Android\Sdk"
.\gradlew.bat testDebugUnitTest lintDebug assembleDebug
Pop-Location
```

Release builds are minified and resource-shrunk:

```powershell
Push-Location mobile-android
.\gradlew.bat testReleaseUnitTest lintRelease assembleRelease
Pop-Location
```

Release signing is intentionally not stored in this repository. Configure a
local keystore and Gradle signing properties before distributing a release
APK. Never commit a keystore, private key, access token or account credential.

## Google Drive setup

1. Create a Google Cloud project and enable Google Drive API v3.
2. Configure an OAuth consent screen. For personal testing, keep the app in
   testing mode and list the rider's Google account as a test user.
3. Create an Android OAuth client for package `com.gpxanimator.mobile`.
4. Add the SHA-1 of every signing certificate that will run the app. Debug and
   release builds normally use different certificates.
5. The app requests only `https://www.googleapis.com/auth/drive.file`.

No OAuth client secret is embedded in an Android application. Google Play
services owns the access-token cache. The app verifies that `drive.file` was
actually granted. An upload worker that needs an account or consent screen sets
the ride to `AUTH_REQUIRED`; its Retry action opens the foreground Google
authorization flow and requeues waiting rides after consent. Disconnect first
invalidates and cancels existing workers, then revokes the grant, so an older
upload cannot mark a ride synced after the user disconnects.

Completed tracks are stored in a visible folder hierarchy:

```text
My Drive/
  GPX Animator/
    Routes/
      2026/
        2026-07-17_12-00_Taipei Ride_12345678.gpx
```

The year folder and filename use the phone time zone captured when the ride
started; GPX timestamps remain UTC. Folder IDs and file IDs are persisted
instead of treating names as unique. Each uploaded item also carries private
app properties for trip ID, schema version and SHA-256 so a retry cannot
silently create duplicate route files. Files up to 5 MiB use multipart upload.
Larger files use a resumable upload whose Google session URL, content hash and
length are kept in the app-private database; after a process or network failure
the worker probes the committed offset and continues instead of retransmitting
the whole route. The session metadata is excluded from backup and cleared when
the upload succeeds or Drive is disconnected.

On Windows, install Google Drive for desktop and make the `GPX Animator/Routes`
folder available locally. Open the synced `.gpx` with GPX Animator as a normal
local file. The desktop application does not need a Google credential.

## Recording lifecycle

The Start button is the only normal entry into a new ride. It is invoked while
the activity is visible and starts a location foreground service. The service
promotes itself immediately, then creates the Room trip with the current boot
counter and local time zone before requesting locations. It is configured not
to stop with the activity task and returns `START_STICKY` so Android can restore
it after ordinary process reclamation.

Every accepted location is committed in one Room transaction together with the
trip summary. The accepted-fix policy requires a valid WGS84 coordinate, a fix
no older than ten seconds and horizontal accuracy within 50 metres. A jump
requiring more than 250 km/h inside a 60-second window is rejected. Timestamps
use the first wall-clock anchor plus monotonic elapsed time, preventing a clock
change from making GPX time move backwards.

Finishing is protected by a press-and-hold confirmation. The service stops
location updates, marks the trip finalizing, and queues an idempotent worker.
The worker writes a temporary GPX, flushes and syncs it, parses the file to
verify the expected track-point count, then performs an atomic rename. Drive
upload starts only after that local file is complete. `FINALIZING` and pending
upload states form a persistent outbox: app startup re-enqueues either stage if
the process died between the database update and WorkManager handoff.

The service refreshes an app-private recording lease. A same-boot, fresh lease
lets a cold UI process recover the service without racing a static in-process
flag. A stale lease or changed boot count marks the ride interrupted rather than
joining points across an unknown gap. Persisted points remain available from
ride details, where the rider can generate, share and optionally sync a partial
GPX.

Android 13 and newer allow a user to press Stop in the system's Active apps
panel, and Android Settings offers a true Force stop action. Android may
terminate the whole app without a callback and forbids automatic restart after
Force stop. The recorder cannot bypass that operating-system boundary. The
expected recovery is preserved points plus an interrupted-ride warning on the
next manual launch, never a fabricated continuous route.

## Permissions

- Precise/coarse location: needed for a useful GPX route.
- Foreground service and foreground-service location: keeps the user-visible
  recording active with another app in front or the screen off.
- Notifications on Android 13+: makes the ongoing recording state visible.
- Internet/network state: used only for optional Drive authorization/upload.

The app does not request `ACCESS_BACKGROUND_LOCATION`, overlay, accessibility,
contacts, activity recognition, microphone, camera or broad storage access.

## Acceptance checklist

Run the following on the actual rider phone before trusting the recorder for a
long trip:

1. Start a ride and wait for at least two accurate fixes.
2. Open Google Maps and the speed-camera app; keep both in normal use.
3. Turn off the display for at least 30 minutes.
4. Remove GPX Animator Ride from Recents and confirm the recording notification
   and point count continue.
5. Restore connectivity after completing a ride offline and verify exactly one
   Drive file appears.
6. Open that GPX in the native Windows animator and compare distance, start/end
   coordinates, timestamp and elevation presence.
7. Repeat once with Battery Saver enabled and record any manufacturer-specific
   unrestricted-battery setting needed by the phone.

Also test deliberate force-stop. The expected result is preserved data and an
interrupted-ride warning on next launch, not an impossible automatic restart.
