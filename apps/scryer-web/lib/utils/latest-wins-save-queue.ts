export type LatestWinsSaveRunner<T> = (value: T) => Promise<void>;

export class LatestWinsSaveQueue<T> {
  private pendingValue: T | undefined;
  private hasPendingValue = false;
  private drainPromise: Promise<void> | null = null;

  enqueue(value: T, runner: LatestWinsSaveRunner<T>): Promise<void> {
    this.pendingValue = value;
    this.hasPendingValue = true;

    if (this.drainPromise) {
      return this.drainPromise;
    }

    this.drainPromise = this.drain(runner);
    return this.drainPromise;
  }

  private async drain(runner: LatestWinsSaveRunner<T>): Promise<void> {
    try {
      while (this.hasPendingValue) {
        const value = this.pendingValue as T;
        this.pendingValue = undefined;
        this.hasPendingValue = false;
        await runner(value);
      }
    } finally {
      this.drainPromise = null;
    }
  }
}
