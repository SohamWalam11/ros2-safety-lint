# rosfix check-bag

    rosfix check-bag BAG [PATH] [--launch FILE] [--format text|json] [-o FILE]

rosfix normally reads files only. check-bag adds one piece of runtime evidence: a recording
of the running system. BAG is a rosbag2 directory, its metadata.yaml, or a text file with the
output of `ros2 topic list -t`. PATH is the workspace, analysed as `rosfix check` would.

The command compares every recorded topic with the launch graph and reports:

    type      the recorded message type differs from the type a node in the code uses
    qos       the recorded publishers offered a different reliability or durability than
              the code declares (a QoS override, a middleware profile, or another publisher
              changed it)
    unknown   the topic was recorded, but no node of the launch graph publishes it (a
              publisher outside the workspace, or a node rosfix could not resolve)
    missing   only with --launch: the selected launch file publishes the topic, but the
              recording does not contain it (the node did not run, or the topic was not
              recorded)

--launch FILE limits the comparison to the launch graph of one launch file (matched by the end
of its path, for example `--launch bringup.launch.py`), which is what you want when the
recording was made while that file was running.

The exit code is 1 when there is a type mismatch, which is always an inconsistency, and 0
otherwise: the other kinds can be explained by what was chosen for recording or by
publishers outside the workspace. A `ros2 topic list -t` file carries no QoS, so only type,
unknown and missing are reported for it.

rosbag2 stores the publishers' QoS as a YAML document embedded in a string; rosfix reads it in
the layouts of metadata versions 5 to 9 seen in practice, with numeric codes (reliability 1 =
reliable, 2 = best effort; durability 1 = transient local, 2 = volatile) or with the policy
names.
