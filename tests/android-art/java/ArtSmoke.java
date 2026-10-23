/** Entry point run by app_process: loads the Rust library and prints the transcript of the mod host run. */
public final class ArtSmoke {
    static native String run(String modsDir);

    public static void main(String[] args) {
        System.load(args[0]);
        System.out.println(run(args[1]));
    }
}
