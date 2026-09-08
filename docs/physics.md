# Physics

Hair, cloth, ribbons, ears, tails and accessories.

## Not a physics engine

This is not rigid-body dynamics. We simulate strands, not boxes, so there are
no contact manifolds, no joints, no continuous collision detection and no
friction model.

    IN                                  OUT
    particles                           rigid bodies
    distance constraints (chains)       joints and motors
    bend constraints (stiffness)        contact manifolds
    collision against primitives        friction models
    attachment / root pinning           continuous collision detection
    compliance (timestep-independent    self-collision
      stiffness)

That list covers everything a puppet needs. Scope creep here turns a
two-thousand-line solver into a research project.

## Position-based, not force-based

    apply external forces
    predict positions
    repeat K times: project every constraint
    derive velocities from the position change
    damp

Force-based springs explode at high stiffness and need the timestep tuned per
rig. Projecting positions instead is unconditionally stable, and it fits the
frame-cost requirement exactly: `K` is a constant, arrays are preallocated, and
collision is just another constraint rather than a special case.

## Two solvers, deliberately

Imported models use a faithful reimplementation of the pendulum-chain model
they were authored against, so they move exactly as their rigger intended.
Native rigs use the full solver with collision.

    import          -> pendulum, bit-faithful
    author natively -> full solver, with collision
    upgrade         -> offered per chain, never forced

A rigger can add collision to the front bangs alone and leave every other chain
identical to how it moved before.

## Colliders are part of the format

A capsule on the head has to turn when the head turns, so a collider is
parented to a deformer exactly as artwork is. That makes it a node in the
format and an authoring mode in the editor, which is why physics could not be
bolted on later without a format break.

## Presets

Presets are not convenience. They are how a rigger's knowledge is packaged and
shared, and they are the difference between "what number goes here" and a
tweak. They are **data**, not built in, so the library belongs to the community.

A preset is parameterised by the art, never hardcoded to it: "given a strand of
N segments rooted at this deformer, use these dynamics". That requires binding
slots in the format, which is a format feature rather than an editor feature.
