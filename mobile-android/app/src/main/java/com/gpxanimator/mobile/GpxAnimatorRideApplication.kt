package com.gpxanimator.mobile

import android.app.Application
import com.gpxanimator.mobile.data.RideDatabase
import com.gpxanimator.mobile.data.RideRepository

class GpxAnimatorRideApplication : Application() {
    val container: AppContainer by lazy { AppContainer(this) }
}

class AppContainer(application: Application) {
    val database: RideDatabase = RideDatabase.create(application)
    val rideRepository = RideRepository(database)
}
