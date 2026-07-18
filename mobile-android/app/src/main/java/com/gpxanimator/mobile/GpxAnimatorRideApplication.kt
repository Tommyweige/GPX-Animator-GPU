package com.gpxanimator.mobile

import android.app.Application
import android.content.Context
import com.gpxanimator.mobile.data.RideDatabase
import com.gpxanimator.mobile.data.RideRepository
import com.gpxanimator.mobile.locale.AppLanguagePreferences
import com.gpxanimator.mobile.recovery.StartupRecoveryCoordinator
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch

class GpxAnimatorRideApplication : Application() {
    val container: AppContainer by lazy { AppContainer(this) }
    private val applicationScope = CoroutineScope(SupervisorJob() + Dispatchers.Default)

    override fun attachBaseContext(base: Context) {
        super.attachBaseContext(AppLanguagePreferences.localizedContext(base))
    }

    override fun onCreate() {
        super.onCreate()
        applicationScope.launch {
            StartupRecoveryCoordinator.recover(
                context = this@GpxAnimatorRideApplication,
                repository = container.rideRepository,
            )
        }
    }
}

class AppContainer(application: Application) {
    val database: RideDatabase = RideDatabase.create(application)
    val rideRepository = RideRepository(database)
}
