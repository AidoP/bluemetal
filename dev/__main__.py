if __name__ == '__main__':
    import argparse
    import dev
    parser = argparse.ArgumentParser(
        description='development tool for bluemetal',
    )

    subparsers = parser.add_subparsers(dest='command')

    run = subparsers.add_parser('run', help='run the nucleus for a given platform')
    run.add_argument('platform', help='the platform to run')

    args = parser.parse_args()
    match args.command:
        case 'run':
            exit(dev.run(args.platform))

    exit(dev.main())
