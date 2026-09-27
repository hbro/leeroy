# TODO

- [ ] **Only currently possible promotions**: the promote overlay should only
  offer steps that can be taken now in the run in view. With a promotion path
  dev → acc → prod where only dev is done and acc waits for its trigger, prod
  can't be promoted yet and shouldn't be listed.

- [ ] **"new run" in the run view**: the run view's `b` hint says "start run";
  label it "new run" there (it's a run already).

- [ ] **Sluggish Pipeline runs list**: moving up/down feels slow. Investigate
  whether every selection change (or frame) does an expensive recomputation,
  e.g. rebuilding all pipelines and runs from the fetched data.
