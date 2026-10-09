# Vanilla 26.3 biome network registry

`vanilla_biome_registry_26_3.bin` is the configuration-phase biome registry packet
payload received from the unmodified vanilla 26.3 server, protocol 777. It retains
the registry identifier, entry count, ordered identifiers, presence flags and
unnamed NBT compounds. The packet id and transport length are excluded.
`vanilla_biome_registry_26_3.json` is its decoded, ordered companion for inspection;
each entry includes its numerical id, identifier and full compound. The binary
is the test oracle because JSON does not preserve NBT numeric types.

## Capture

The capture used `F:\minecraft-rust\vanilla\jar\server-bundler.jar`, SHA-256
`d052f14d7a173734fba553711e5b570162e2f2a313267ee31a21b975a679be64`,
and `F:\minecraft-rust\jdk-25\bin\java.exe`. The server ran in
`F:\minecraft-rust\scratch\lane-e-village\registry-network`, with no added
datapacks, `eula=true`, `online-mode=false`, `white-list=false`, port 25599,
view and simulation distances 2, and `network-compression-threshold=-1`.
It received `stop` after capture and exited with status 0. No server remains running.

The scratch script `F:\minecraft-rust\scratch\lane-e-village\capture_registry.py`
implements a small TCP client using Python's standard library:

1. Send a protocol-777 login handshake and login hello.
2. Receive login finished and send login acknowledged.
3. Receive configuration select-known-packs and reply with an empty pack list.
4. Read registry-data packets and retain `minecraft:worldgen/biome`.
5. Decode all 67 ordered identifiers, data-presence flags and unnamed NBT compounds;
   require complete consumption of the packet payload. Close the client and stop
   the server. The scratch directory retains the raw packet, decoded JSON and log.

An empty known-pack list forces `SynchronizeRegistriesTask.handleResponse` /
`sendRegistries` to call `RegistrySynchronization.packRegistries` with no skipped
contents. `RegistryDataLoader.SYNCHRONIZED_REGISTRIES` selects `Biome.NETWORK_CODEC`
(`Biome.java:49-55`). That codec selects climate settings, effects and
`EnvironmentAttributeMap.NETWORK_CODEC`; `filterSyncable`
(`EnvironmentAttributeMap.java:23-25,36-37`) removes non-syncable attributes.
This fixture therefore records the wire encoding, not raw datapack JSON.

The retained payload is 20,880 bytes, SHA-256
`273a4803dd79b8114c10ca1989627e96fe1ccdebd53f965afb26103aba3a2b65`.
Entry ids are positions in the captured list, 0 through 66. The test compares
those names and positions to the packet Pumpkin writes and to `Biome.id`.

## Differences exposed and corrected

All 67 biomes previously sent extra `carvers`, `features` and
`attributes/minecraft:gameplay/natural_mob_spawns`. Their `temperature` and
`downfall` were NBT doubles rather than vanilla's floats. Additional differences:

| Biomes | Field | Correction |
| --- | --- | --- |
| badlands, eroded_badlands, ice_spikes, snowy_plains, wooded_badlands | attributes/minecraft:gameplay/creature_world_gen_spawn_probability | Omit non-syncable attribute |
| badlands, desert, eroded_badlands, savanna, savanna_plateau, windswept_savanna, wooded_badlands | attributes/minecraft:gameplay/snow_golem_melts | Omit non-syncable attribute |
| bamboo_jungle, frozen_peaks, jagged_peaks, jungle, mangrove_swamp, mushroom_fields, snowy_slopes, swamp | attributes/minecraft:gameplay/increased_fire_burnout | Omit non-syncable attribute |
| mushroom_fields | attributes/minecraft:gameplay/can_pillager_patrol_spawn | Omit non-syncable attribute |
| basalt_deltas, crimson_forest, soul_sand_valley, warped_forest | attributes/minecraft:visual/ambient_particles/argument/0/probability | Encode float instead of double |
| mangrove_swamp, swamp | attributes/minecraft:visual/water_fog_end_distance/argument | Encode float instead of double |
| pale_garden | attributes/minecraft:audio/music_volume | Encode float instead of double |

No `temperature_modifier` values differed: only `deep_frozen_ocean` and
`frozen_ocean` send `frozen`; the default is absent in every other entry.
Codegen also omits explicit `none` defaults, matching
`Biome.ClimateSettings.CODEC` and `BiomeSpecialEffects.CODEC`.
Ambient sound mood offsets and addition chances remain doubles, as required by
`AmbientMoodSettings.CODEC` and `AmbientAdditionsSettings.CODEC`.

`biome_configuration_packet_matches_vanilla_26_3` recursively compares the entire
decoded compound, including key absence, list lengths, numeric tag types and
values. Extra non-syncable attributes and incorrect temperature modifiers fail.
The old partial raw-JSON comparison has been replaced. These checks establish
decoded network parity for these default biome entries, not client rendering,
custom datapacks or byte equality of unordered compound keys.

The stronger packet test failed against the original generated serialization.
Scratch mutation checks then injected a non-syncable attribute into decoded
packet data and an incorrect `frozen` temperature modifier into plains; each
failed independently. `explicit_default_biome_modifiers_are_omitted` also failed
with default omission disabled. All temporary changes were restored and both
tests passed. Logs are retained in the capture scratch directory.
