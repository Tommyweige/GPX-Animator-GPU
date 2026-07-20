package com.gpxanimator.mobile.data

import android.content.Context
import androidx.room.Database
import androidx.room.Room
import androidx.room.RoomDatabase
import androidx.room.TypeConverter
import androidx.room.TypeConverters

class RideConverters {
    @TypeConverter
    fun tripStateToString(value: TripState): String = value.name

    @TypeConverter
    fun stringToTripState(value: String): TripState = TripState.valueOf(value)

    @TypeConverter
    fun syncStateToString(value: SyncState): String = value.name

    @TypeConverter
    fun stringToSyncState(value: String): SyncState = SyncState.valueOf(value)
}

@Database(
    entities = [TripEntity::class, TrackPointEntity::class],
    version = 1,
    exportSchema = true,
)
@TypeConverters(RideConverters::class)
abstract class RideDatabase : RoomDatabase() {
    abstract fun tripDao(): TripDao

    abstract fun trackPointDao(): TrackPointDao

    companion object {
        fun create(context: Context): RideDatabase =
            Room.databaseBuilder(
                context.applicationContext,
                RideDatabase::class.java,
                "ride-recordings.db",
            ).build()
    }
}
