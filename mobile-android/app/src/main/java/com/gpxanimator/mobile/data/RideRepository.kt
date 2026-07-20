package com.gpxanimator.mobile.data

import androidx.room.withTransaction
import kotlinx.coroutines.flow.Flow

class RideRepository(private val database: RideDatabase) {
    val trips: Flow<List<TripEntity>> = database.tripDao().observeAll()

    suspend fun create(trip: TripEntity) = database.tripDao().insert(trip)

    suspend fun getTrip(tripId: String): TripEntity? = database.tripDao().get(tripId)

    suspend fun getActiveTrip(): TripEntity? = database.tripDao().getActive()

    fun observeTrip(tripId: String): Flow<TripEntity?> = database.tripDao().observe(tripId)

    suspend fun points(tripId: String): List<TrackPointEntity> =
        database.trackPointDao().list(tripId)

    suspend fun lastPoint(tripId: String): TrackPointEntity? =
        database.trackPointDao().last(tripId)

    suspend fun appendPoint(point: TrackPointEntity, trip: TripEntity) {
        database.withTransaction {
            database.trackPointDao().insert(point)
            database.tripDao().update(trip)
        }
    }

    suspend fun updateTrip(trip: TripEntity) = database.tripDao().update(trip)

    suspend fun deleteTrip(tripId: String) = database.tripDao().delete(tripId)
}
