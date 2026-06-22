## Fitted Robots (Scene Manifest) Requirements

Scene manifests define complete robotic cell layouts as YAML files. Each manifest declares which dressed robots and fixtures to place, at what world-space poses, and which floor plan to use as a reference overlay.

### What qualifies as a fitted-robots manifest

A scene manifest that `robo-fitting-room` can validate, compose, and deploy:
- One or more dressed robots (output of `urdf-robo-dresser`)
- Zero or more fixtures (from `urdf-fixture-collection`)
- Optional 2D CAD floor plan (from `floor-plans/`)

### Naming Convention

`<CELL_NAME>-cell.yaml`

### Path Convention

All paths in the manifest are relative to the manifest file itself. Assets are referenced via relative paths into the sibling asset directories:

```
assets/
├── fitted-robots/
│   └── k8urw11-cell.yaml          ← manifest
├── urdf-dressed-robots/            ← dressed_urdf paths point here
├── urdf-fixture-collection/        ← fixture urdf paths point here
└── floor-plans/                    ← floor plan geometry_file points here
```

### Note

Scene manifests referencing customer data are not checked in here.
