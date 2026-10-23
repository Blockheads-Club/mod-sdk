// SPDX-License-Identifier: MIT OR Apache-2.0
package club.blockheads.mod;

import club.blockheads.api.channels.Channels;
import club.blockheads.api.channels.ChannelsImpl;
import club.blockheads.api.channels.Target;
import club.blockheads.api.commands.CommandSpec;
import club.blockheads.api.commands.Commands;
import club.blockheads.api.commands.CommandsImpl;
import club.blockheads.api.events.CustomEvent;
import club.blockheads.api.events.EventHandlers;
import club.blockheads.api.events.EventKind;
import club.blockheads.api.events.Events;
import club.blockheads.api.events.EventsImpl;
import club.blockheads.api.host.Host;
import club.blockheads.api.host.HostImpl;
import club.blockheads.api.log.Level;
import club.blockheads.api.log.Log;
import club.blockheads.api.log.LogImpl;
import club.blockheads.api.players.Players;
import club.blockheads.api.players.PlayersImpl;
import club.blockheads.api.players.Role;
import club.blockheads.api.scheduler.Scheduler;
import club.blockheads.api.scheduler.SchedulerImpl;
import club.blockheads.api.services.Services;
import club.blockheads.api.services.ServicesImpl;
import club.blockheads.api.storage.Storage;
import club.blockheads.api.storage.StorageImpl;
import club.blockheads.api.types.Priority;
import club.blockheads.api.ui.Ui;
import club.blockheads.api.ui.UiImpl;
import club.blockheads.mod.rt.Bridge;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.Map;

/**
 * Everything a mod can do: the raw WIT interfaces ({@link #host()}, {@link #players()}, ...) and shortcuts that also
 * keep the callbacks ({@link #onChat}, {@link #command}, {@link #runLater}, {@link #provide}, ...).
 *
 * <p>Handlers run on the game thread inside a host call; keep them short (soft budget 5 ms, hard budget 25 ms).
 */
public final class Api implements EventHandlers {
    private final Bridge bridge;
    private final Host host;
    private final Log log;
    private final Players players;
    private final Events events;
    private final Commands commands;
    private final Scheduler scheduler;
    private final Storage storage;
    private final Services services;
    private final Channels channels;
    private final Ui ui;

    final Map<Long, Handler<Object>> handlers = new HashMap<Long, Handler<Object>>();
    final Map<Long, CommandHandler> commandHandlers = new HashMap<Long, CommandHandler>();
    final Map<Long, Object[]> tasks = new HashMap<Long, Object[]>();
    final Map<String, Service> providers = new HashMap<String, Service>();
    final Map<Long, ChannelHandler> channelHandlers = new HashMap<Long, ChannelHandler>();

    /** Used by the runtime; mods receive their instance in {@link Mod#onEnable}. */
    public Api(Bridge b) {
        bridge = b;
        host = new HostImpl(b);
        log = new LogImpl(b);
        players = new PlayersImpl(b);
        events = new EventsImpl(b);
        commands = new CommandsImpl(b);
        scheduler = new SchedulerImpl(b);
        storage = new StorageImpl(b);
        services = new ServicesImpl(b);
        channels = new ChannelsImpl(b);
        ui = new UiImpl(b);
    }

    /**
     * The raw connection to the host: {@code bridge().call("interface.function", args...)} calls any WIT import by
     * name, also those a newer host has and this SDK has no wrapper for yet. Arguments and results are plain CBOR
     * values (see {@link Bridge#call}).
     */
    public Bridge bridge() {
        return bridge;
    }

    public Host host() {
        return host;
    }

    public Log log() {
        return log;
    }

    public Players players() {
        return players;
    }

    /** The raw {@code events} interface; prefer {@link #onEvent} and the {@code onXxx} shortcuts. */
    public Events events() {
        return events;
    }

    public Commands commands() {
        return commands;
    }

    public Scheduler scheduler() {
        return scheduler;
    }

    public Storage storage() {
        return storage;
    }

    public Services services() {
        return services;
    }

    public Channels channels() {
        return channels;
    }

    /** HUD widgets, toasts, notices and dialogs (API 0.3, client mods; servers answer {@code unsupported}). */
    public Ui ui() {
        return ui;
    }

    // -- logging -----------------------------------------------------------------------------------------------

    public void info(String message) {
        log.log(Level.INFO, message);
    }

    public void warn(String message) {
        log.log(Level.WARN, message);
    }

    public void error(String message) {
        log.log(Level.ERROR, message);
    }

    public void debug(String message) {
        log.log(Level.DEBUG, message);
    }

    // -- events ------------------------------------------------------------------------------------------------

    @Override
    @SuppressWarnings("unchecked")
    public int onEvent(EventKind kind, Priority priority, boolean ignoreCancelled, Handler<?> h) {
        long id = events.subscribe(kind, priority, ignoreCancelled);
        handlers.put(id, (Handler<Object>) h);
        return (int) id;
    }

    /** Subscribes to the custom event {@code name} ({@code <mod-id>:<name>}) of any mod. */
    public int onCustom(String name, Handler<CustomEvent> h) {
        return onCustom(name, Priority.NORMAL, false, h);
    }

    /** Subscribes to a custom event with a priority. */
    @SuppressWarnings("unchecked")
    public int onCustom(String name, Priority priority, boolean ignoreCancelled, Handler<CustomEvent> h) {
        long id = events.subscribeCustom(name, priority, ignoreCancelled);
        handlers.put(id, (Handler<Object>) (Handler<?>) h);
        return (int) id;
    }

    /** Removes a subscription made with {@code onXxx}. */
    public boolean off(int subscription) {
        handlers.remove((long) subscription);
        return events.unsubscribe(subscription);
    }

    /** Fires the custom event {@code name} (must start with this mod's id and a colon). True if it ended up cancelled. */
    public boolean fireCustom(String name, Object payload) {
        return events.fireCustom(name, payload);
    }

    // -- commands ----------------------------------------------------------------------------------------------

    /** Registers {@code /name} for every player and the console. */
    public long command(String name, String description, CommandHandler h) {
        return command(new CommandSpec(name, new ArrayList<String>(), description, "/" + name, Role.PLAYER, true), h);
    }

    public long command(CommandSpec spec, CommandHandler h) {
        long id = commands.register(spec);
        commandHandlers.put(id, h);
        return id;
    }

    public boolean unregisterCommand(long id) {
        commandHandlers.remove(id);
        return commands.unregister(id);
    }

    // -- scheduler ---------------------------------------------------------------------------------------------

    /** Runs {@code t} once after {@code delay} ticks (0.2 s each on the server). */
    public long runLater(int delay, Task t) {
        long id = scheduler.runLater(delay);
        tasks.put(id, new Object[] {t, Boolean.FALSE});
        return id;
    }

    /** Runs {@code t} after {@code delay} ticks and then every {@code period} ticks. */
    public long runEvery(int delay, int period, Task t) {
        long id = scheduler.runEvery(delay, period);
        tasks.put(id, new Object[] {t, Boolean.TRUE});
        return id;
    }

    /** Runs {@code t} once in the next network poll. */
    public long runNextPoll(Task t) {
        long id = scheduler.runNextPoll();
        tasks.put(id, new Object[] {t, Boolean.FALSE});
        return id;
    }

    public boolean cancel(long task) {
        tasks.remove(task);
        return scheduler.cancel(task);
    }

    // -- services ----------------------------------------------------------------------------------------------

    /** Provides a service declared in {@code mod.toml} {@code [provides]}. */
    public void provide(String name, String version, Service impl) {
        services.provide(name, version);
        providers.put(name, impl);
    }

    /** A client for a service of another mod; {@code requirement} is a semver requirement such as {@code ^1}. */
    public ServiceClient service(String name, String requirement) {
        return new ServiceClient(services, name, requirement);
    }

    // -- channels ----------------------------------------------------------------------------------------------

    /** Opens (or joins) the channel {@code name}; {@code h} receives its messages. */
    public long openChannel(String name, ChannelHandler h) {
        long id = channels.open(name);
        channelHandlers.put(id, h);
        return id;
    }

    public void send(long channel, Target to, byte[] data) {
        channels.send(channel, to, data);
    }

    public boolean closeChannel(long channel) {
        channelHandlers.remove(channel);
        return channels.close(channel);
    }
}
