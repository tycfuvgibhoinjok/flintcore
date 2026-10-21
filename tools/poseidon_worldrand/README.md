# The random of the world (`World.random`)

Java `World.java:51`: `public Random random = new Random();` — the seed of the
random of the world comes from the system clock, so the reference is not
reproducible between two runs: the weather, the angle of a spawned mob, the
drops of a block and the random ticks draw different numbers every time.

The port keeps the world reproducible (`world.random_seed = "fixed"`, the
seed 0) and can imitate the reference (`world.random_seed = "clock"`).  The
harness of this directory makes the *reference* reproducible as well: it sets
the seed of the random of the world explicitly, so the draws of the reference
and of the port can be compared number by number.

## What is compared

The golden file `tests/golden/poseidon_world_random.txt` holds the draws of
the random of the world after a scripted sequence of the calls that use it:

| Step | Java | What it draws |
|---|---|---|
| `drop_stone` | `Block.dropNaturally` | the chance of the item and the position of the item entity (4 numbers) |
| `drop_gravel` | `Block.dropNaturally` | the chance, the item (the flint) and the position (5 numbers) |
| `drop_leaves` | `Block.dropNaturally` | the number of the items (`Block.a(Random)`) |
| `drop_crops` | `Block.dropNaturally` | the chance, the position and the three seeds with their positions (16 numbers) |
| `drop_log` | `Block.dropNaturally` | the chance and the position (4 numbers) |
| `weather` | `World.i()` | the durations of the thunder and of the rain (2 numbers) |

Every number is written with the number of the bits of the draw
(`nextInt(31)` of a bound and `nextInt(32)` of a plain number give different
values).

## Regenerate the golden file

```sh
sh tools/poseidon_worldrand/regenerate_golden.sh
```

The test that compares the port with the golden file is
`the_random_of_the_world_draws_the_same_numbers_as_the_reference`
(`src/world/world.rs`).
