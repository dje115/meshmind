# MeshMind On-Device Training

## Bounds

All training jobs have hard caps:
- max_steps, max_minutes, max_dataset_items, max_threads

## Flow

1. Build dataset manifest (event hash range + CAS refs)
2. Run bounded training job
3. Evaluation gate: must beat baseline + regression prompts
4. Publish model bundle as MODEL_BUNDLE artifact
5. Emit TRAIN_JOB_STARTED / TRAIN_JOB_COMPLETED events

## Rollback

Instant rollback to any previous model version via MODEL_ROLLED_BACK event.

## V1 Target

Router/classifier training; CPU-feasible workloads only.
