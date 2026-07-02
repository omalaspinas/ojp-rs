# Tests performed here

There are 8 different test corresponding to the XML files obtained using the examples on <https://opentdatach.github.io/api-explorer2/#/default/OJP2.0>.

We get the 9 examples and store them respectively in 8 files:

1. `location_simple.xml`
2. `location_topographic.xml`
3. `location_coordinate.xml`
4. `location_extended.xml`
5. `stop_simple.xml`
6. `stop_complex.xml`
7. `trip_simple.xml`
8. `trip_lots.xml`

The corresponding requests can be found in the files:

1. `req_location_simple.xml`
2. `req_location_topographic.xml`
3. `req_location_coordinate.xml`
4. `req_location_extended.xml`
5. `req_stop_simple.xml`
6. `req_stop_complex.xml`
7. `req_trip_simple.xml`
8. `req_trip_lots.xml`

## Individual transport (monomodal) trips

`trip_bicycle.xml` (request: `req_trip_bicycle.xml`) was captured directly from the
live endpoint (`https://api.opentransportdata.swiss/ojp20`) on 2026-07-02. The request
is a regular `OJPTripRequest` between two stop refs (Zürich HB 8503000 → Zürich Oerlikon
8503006) with an `<ItModeToCover><PersonalMode>bicycle</PersonalMode></ItModeToCover>`
in `<Params>`. The response contains 4 public-transport trips plus one monomodal
bicycle trip made of a single `<ContinuousLeg>` (note: its `TripResult`/`Trip` id is
the literal string `n/a`, and the leg carries neither `LegTrack` nor `PathGuidance`).

`trip_car.xml`, `trip_motorcycle.xml` and `trip_scooter.xml` (requests:
`req_trip_<mode>.xml`) were captured the same way over the same stop pair and are
structurally identical to the bicycle fixture — 4 public-transport trips plus one bare
monomodal `<ContinuousLeg>`; only the mode string and durations differ.

`trip_truck.xml` (request: `req_trip_truck.xml`) pins the opposite case: the endpoint
silently ignores `<PersonalMode>truck</PersonalMode>` and the response contains only
the 4 public-transport trips.

`trip_foot.xml` (request: `req_trip_foot.xml`) was captured the same way with
`<PersonalMode>foot</PersonalMode>` over a short pair (Zürich HB 8503000 →
Zürich, Sihlquai/HB 8591368, 413 m). Unlike the other monomodal legs, the walk
`<ContinuousLeg>` **does** carry a `<LegTrack>` and a `<PathGuidance>` with turn-by-turn
sections and coordinate polylines, so this fixture pins the richer leg variant
(including repeated `TrackSection` elements, which the schema allows unboundedly).

## Generated (composite) SLOID refs

`trip_generated_sloid.xml` (request: `req_trip_generated_sloid.xml`) is a plain
public-transport `OJPTripRequest` (no `ItModeToCover`) between Le Landeron 8500994 and
Alle, Grands Prés 8574834, captured from the live endpoint on 2026-07-02. Some of its
`StopPointRef`s are generated platform-level composites like
`ch:1:sloid:9068_gen:ch:1:sloid:9068:0:168099_pf:3CD`, where the fourth `:`-separated
part is `9068_gen` (parent stop's SLOID number plus a `_gen` suffix). This fixture pins
that `sloid_to_didok` handles such refs (regression: they used to fail with "invalid
digit found in string").

Observed endpoint behaviour (not schema-mandated):

- Only one `<ItModeToCover>` per request is honoured; when several are given, only one
  monomodal trip comes back (the `car` one, when `bicycle` and `car` were both requested).
- Monomodal trips are returned for `foot`, `bicycle`, `car`, `motorcycle` and `scooter`.
- `foot` is subject to a walking-distance policy: 413 m works, ~1.5 km and ~4.7 km pairs
  return only the public-transport trips (no error).
- `bicycle` and `scooter` have (larger) distance limits as well: both work at ~4.7 km but
  return nothing for an intercity pair (~180 km), where `car` and `motorcycle` still do.
- `truck` is silently ignored: valid response, public-transport trips only.
- With coordinate (`GeoPosition`) origins/destinations instead of stop refs, walk access
  legs appear as `ContinuousLeg`s whose `LegStart`/`LegEnd` have **no** `StopPointRef`,
  only `GeoPosition` — not currently supported by the deserializer.
