package com.gpxanimator.mobile.data

import androidx.room.Dao
import androidx.room.Insert
import androidx.room.OnConflictStrategy
import androidx.room.Query
import androidx.room.Update
import kotlinx.coroutines.flow.Flow

@Dao
interface TripDao {
    @Insert(onConflict = OnConflictStrategy.ABORT)
    suspend fun insert(trip: TripEntity)

    @Update
    suspend fun update(trip: TripEntity)

    @Query("SELECT * FROM trips WHERE id = :tripId LIMIT 1")
    suspend fun get(tripId: String): TripEntity?

    @Query("SELECT * FROM trips WHERE state = 'RECORDING' ORDER BY startedAtEpochMillis DESC LIMIT 1")
    suspend fun getActive(): TripEntity?

    @Query("SELECT * FROM trips ORDER BY startedAtEpochMillis DESC")
    fun observeAll(): Flow<List<TripEntity>>

    @Query("SELECT * FROM trips WHERE id = :tripId LIMIT 1")
    fun observe(tripId: String): Flow<TripEntity?>

    @Query("DELETE FROM trips WHERE id = :tripId")
    suspend fun delete(tripId: String)
}

@Dao
interface TrackPointDao {
    @Insert(onConflict = OnConflictStrategy.ABORT)
    suspend fun insert(point: TrackPointEntity)

    @Query("SELECT * FROM track_points WHERE tripId = :tripId ORDER BY sequence")
    suspend fun list(tripId: String): List<TrackPointEntity>

    @Query("SELECT * FROM track_points WHERE tripId = :tripId ORDER BY sequence DESC LIMIT 1")
    suspend fun last(tripId: String): TrackPointEntity?

    @Query("SELECT COUNT(*) FROM track_points WHERE tripId = :tripId")
    suspend fun count(tripId: String): Int
}
