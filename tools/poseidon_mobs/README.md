# Poseidon mob trace harness

`PoseidonMobTrace.java` runs a single mob of the reference implementation
(Project-Poseidon, Beta 1.7.3) in a handcrafted world and dumps its state
after every tick:

```
tick x y z motX motY motZ yaw pitch onGround fallDistance target pathLen pathHash
```

The dump is the golden trace of the mob movement: the flintcore port has to
reproduce it tick by tick (the comparison lives in
`src/server.rs`, the test `the_mob_movement_matches_the_poseidon_reference`,
and the golden file in `tests/golden/poseidon_mob_traces.txt`).

## Build and run

The reference sources have to be compiled once into `target/poseidon_classes`
(see `DEVELOPMENT.md` section 6), then:

```sh
javac -nowarn -cp "target/poseidon_classes:.references/Project-Poseidon/libs/*" \
    -d target/poseidon_classes tools/poseidon_mobs/PoseidonMobTrace.java

for s in flat chase water wall lava slime squid ghast; do
  java -cp "target/poseidon_classes:.references/Project-Poseidon/libs/*" \
    flintcore.test.PoseidonMobTrace $s 12345 /tmp/mob_traces/$s.txt 300
done
```

The traces are written to `/tmp/mob_traces/<scenario>.txt` and are assembled
into the golden file by `tools/poseidon_mobs/regenerate_golden.sh` (the
header of the golden file documents the command).

## The scenarios

| scenario | mob      | world                                              |
|----------|----------|----------------------------------------------------|
| `flat`   | pig      | a flat grass floor, the mob wanders               |
| `chase`  | zombie   | a one block step between the mob and the player     |
| `water`  | zombie   | a pool of water between the mob and the player      |
| `wall`   | spider   | a wall between the mob and the player               |
| `lava`   | zombie   | a pool of lava between the mob and the player      |
| `slime`  | slime    | a flat stone floor, the slime hops                 |
| `squid`  | squid    | a tank of water                                     |
| `ghast`  | ghast    | a flat stone floor, the ghast flies                 |
| `creeper`| creeper  | a flat floor, the creeper walks to the player       |
| `skeleton`| skeleton| a flat floor, the skeleton shoots at the player     |
| `chicken`| chicken  | a flat grass floor, the chicken wanders             |
| `cow`    | cow      | a flat grass floor, the cow wanders                 |
| `sheep`  | sheep    | a flat grass floor, the sheep wanders               |
| `wolf`   | wolf     | a flat grass floor, the wolf walks to the player    |
| `pigzombie` | pig zombie | a flat floor, the mob walks to the player       |
| `giant`  | giant zombie | a flat floor, the mob walks to the player        |

## Notes

- Poseidon shares **one** `Random` between all entities
  (`Entity.SHARED_RANDOM`, a backport of the PaperSpigot patch). The seed is
  taken from the system clock, so the harness calls
  `Entity.SHARED_RANDOM.setSeed(seed)` — the first call wins and locks the
  stream, without it the traces are not reproducible.
- The world is created with the same trick as `tools/poseidon_dump`: the
  `WorldServer` is allocated with `Unsafe` and the CraftBukkit server is set
  up by hand, because the CraftBukkit events of the AI need it.
- `world.isLoading` stays `true` while the scenario is built: CraftBukkit
  hands out the empty chunk otherwise and the terrain would be lost.
- The light of the scenario is set by hand (`scenarioLight`): the world of
  the harness is generated with the normal generator, so the light of the
  hills around the floor would be an artefact of the terrain. The port sets
  the same light, otherwise the mobs would walk to a different goal. The
  light of a column is written for the whole height of the chunk (`y` from 0
  to 127): a bigger loop overflows the index of the `NibbleArray` into the
  neighbouring column and lights the world in a way the reference never
  does.
- A mob that died is not updated any more: the world removes the dead
  entities at the end of its tick (`World.tickEntities`), so the harness
  stops the mob when the counter of the death (`EntityLiving.deathTicks`)
  passes twenty and prints the frozen state of the mob. The flag `dead` of
  the entity cannot be used: the monsters of the harness die at once
  (`EntityMonster.m_()`: the world of the harness is in the peaceful mode).
- The floor of the scenarios is grass: the animals score the blocks of their
  walk with the grass below them (`EntityAnimal.a`), so the goal of the walk
  does not depend on the light.
