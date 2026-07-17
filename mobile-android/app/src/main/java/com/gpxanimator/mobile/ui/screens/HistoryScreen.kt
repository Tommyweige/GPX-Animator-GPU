package com.gpxanimator.mobile.ui.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.gpxanimator.mobile.R
import com.gpxanimator.mobile.ui.RideUiState
import com.gpxanimator.mobile.ui.components.EmptyStateCard
import com.gpxanimator.mobile.ui.components.PageIntro
import com.gpxanimator.mobile.ui.components.RideListRow
import com.gpxanimator.mobile.ui.theme.RideTeal

@Composable
fun HistoryScreen(
    state: RideUiState,
    onOpenTrip: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    LazyColumn(
        modifier = modifier.fillMaxSize(),
        contentPadding = androidx.compose.foundation.layout.PaddingValues(
            20.dp,
            24.dp,
            20.dp,
            32.dp
        ),
        verticalArrangement = Arrangement.spacedBy(14.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        item {
            PageIntro(
                eyebrow = stringResource(R.string.nav_history),
                title = stringResource(R.string.history_title),
                subtitle = stringResource(R.string.history_subtitle),
                modifier = Modifier.padding(bottom = 6.dp),
            )
        }
        when {
            state.isLoading -> item { CircularProgressIndicator(color = RideTeal) }
            state.hasDataError ->
                item {
                    EmptyStateCard(
                        title = stringResource(R.string.data_unavailable_title),
                        body = stringResource(R.string.data_unavailable_body),
                    )
                }

            state.trips.isEmpty() ->
                item {
                    EmptyStateCard(
                        title = stringResource(R.string.no_rides_title),
                        body = stringResource(R.string.no_rides_body),
                    )
                }

            else ->
                items(items = state.trips, key = { it.id }) { trip ->
                    RideListRow(trip = trip, onClick = { onOpenTrip(trip.id) })
                }
        }
    }
}
