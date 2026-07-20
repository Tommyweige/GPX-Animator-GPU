# Room and Google Play services ship consumer rules. Keep only GPX XML model names
# because exported extension element names are part of the public file contract.
-keepclassmembers enum com.gpxanimator.mobile.data.TripState { *; }
-keepclassmembers enum com.gpxanimator.mobile.data.SyncState { *; }
